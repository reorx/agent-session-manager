---
created: 2026-10-26
tags:
  - favorites
  - persistence
  - acceptance
---

# Session 收藏实现与验收

## 概要

实现独立快照收藏，避免 Claude Code 清理原文件后丢失正文。收藏主文件和附件一起保存，扫描按 session id 收藏优先；取消收藏移至废纸篓，最后副本要求确认。CLI、菜单、⌘D、行右键、详情星标和仅收藏过滤均已接入。

## 修改的文件

- core `config.rs`、`favorites.rs`：共享目录配置、保留 mtime 的快照、半行截断、废纸篓与迁移回滚。
- core discovery、schema、db、indexer：schema 3、收藏标记、两处发现去重、路径切换 Full 解析、根目录守卫。
- CLI、transcript、regex：收藏管理命令，直接扫描和导出也读取收藏。
- app store、favorites、model、settings、table、toolbar、detail、main：后台串行文件操作、界面入口、提示、过滤持久化与焦点恢复。
- core / app 测试：收藏状态转换、费用与消息一致性、目录迁移、最后副本保护和键盘行为。
- 方案、产品状态、数据格式、运行时架构、AGENTS.md 与 todo 同步更新。

## 验证

- `mise exec -- make test` 全部通过；app 80 项通过，1 项性能探针按既有规则忽略；收藏专项 19 项通过。
- `mise exec -- make parity`：21 组通过，0 差异。
- 真实数据验收：2,038 份 transcript；目标快照 3 个文件、2,869,518 字节，内容和纳秒级 mtime 一致。临时数据上的完整 `asm favorite` 命令耗时 22.85ms，不能据此断言使用 clonefile 或测得纯复制时间。
- 实机已验证星标、仅收藏过滤、详情按钮、右键菜单、原文件删除后仍为 CLI 类型且可搜索、最后副本原生确认框，以及 CLI 收藏往返、`--yes`、目录迁移和恢复默认目录。确认取消及焦点恢复由无界面键盘测试覆盖；实机鼠标取消未确认生效。
- 删除临时原文件后保留 16 条索引消息、1 个 subagent、$10.952702 费用。
- 截图已打开检查，归档 `tmp/2026-10-26-favorites/`，完整报告在其 `report.md`。测试应用已退出，未触碰真实原文件或系统废纸篓。用户确认验收已足够，停止新增验收。

## 注意事项

- shell 默认 Rust 版本较旧，使用 `mise exec --` 调用项目锁定的工具链。
- macOS 的 `/var` 和 `/private/var` 为同一路径；迁移和根目录身份比较使用 canonical 路径。
- 只有已落位主 JSONL 的收藏参与覆盖；保存时拒绝孤立同名附件目录，防止把旧 subagent 接到新快照。该边界先复现再修复，回归测试通过。
- 目录迁移完成后再保存配置和索引根目录，配置失败也回滚已迁移文件。
- 实机验收按项目 context 规则使用 Herdr 新 tab 的 Codex 完成，主 agent 复核截图；验收 agent 未修改源码。

## 遗留问题

本次范围内无已知未解决问题。跨卷迁移的 EXDEV 分支和 APFS clonefile 是否实际使用未做跨卷或系统调用级实测。快照刷新、恢复到 Claude 原目录和 iCloud 占位文件行为仍属于方案列出的后续范围。

## 相关文档

- [收藏方案](../plans/2026-10-26-favorites.md) — 本次依据并保存。
- [运行时架构](../docs/app-architecture.md) — 更新收藏数据流和目录守卫。
- [数据格式](../docs/claude-session-data-format.md) — 更新收藏布局与优先规则。
- [产品状态](../docs/product-prd.md) — 更新收藏功能状态。
- [GPUI 开发指南](../docs/gpui-kit-guide.md) — 界面、焦点和测试的依据。
