---
created: 2026-11-03
tags:
  - context-window
  - model-catalog
  - acceptance
  - agent-session-manager
---

# Context 窗口接入 Claude Code 本机模型目录

## 概要

按计划将 Context 窗口改为优先读 Claude Code 的本机模型目录，缺失时退回内置表。表格百分比与排序、Metadata、`asm cost` 使用同一个显式传入的目录；启动和扫描后读取，窗口表变化时即使 session 没变化也刷新整表。价格、schema、索引版本与持久化配置不变，无需重建索引。

`asm cost --windows` 列出读取或跳过的文件、各模型的目录与内置窗口、受影响的 session 数，并按更新时间列出变化的 session；复用费用查询的项目、日期和数量过滤，与 `--compare` 互斥。

## 修改的文件

- `crates/core/src/claude/model_catalog.rs`、`paths.rs`、`mod.rs`：类型化读取 served / published 的 cc surface，按来源和获取时间合并，归一化、校验窗口，集中处理读取错误与诊断。
- core `pricing.rs`、`model.rs`、`bin/asm.rs`：目录优先的窗口与比例查询，CLI 输出和排序，以及 `--windows` 对比。
- app `store.rs`、`main.rs`、`model.rs`：目录在后台加载，通过 `Loaded` 传到主线程，统一重建行；仅比较窗口表，获取时间变化不刷新。
- core `tests/model_catalog.rs`、`tests/pricing.rs`、`tests/cli.rs`、fixture 生成器与生成的模型目录：行为覆盖；app 行模型、store 与无界面窗口测试覆盖目录刷新。actions / table 的原有测试通过共享辅助函数传空目录。
- `AGENTS.md`、产品 PRD、数据格式、运行时架构、旧费用计划和已知问题：更新窗口来源、生效时机、诊断命令与限制。

## 验证

先留旧版 release CLI、索引副本和启动日志，再写行为测试；core 与 app 的预期失败日志均保存在验收目录。`make test` 全部通过（app 132 passed，2 个现有性能探针 ignored）；整理测试辅助函数后再次跑 app 全部通过。未运行 formatter、lint 或 `make parity`：没有改 JSONL 解析、清理、搜索或列表输出。

覆盖的边界包括新模型、内置回退、目录覆盖、日期和 `[1m]`、大小写和空白归一化、同文件 canonical id 优先、多文件新旧与 served / published 优先级、过期缓存、坏 JSON、缺字段、非法窗口、无目录及忽略非目标文件。store 测试验证第一份 `Loaded` 自带目录，以及仅窗口变化才刷新（包含删除回退）。无界面窗口测试实际向 store 发 Scan，确认表格比例和选中行 Metadata 同时从内置 1M 切到目录 100K，选中保持不变。

### 真实数据与耗时

- CLI 固定索引副本含 2304 个 session，1581 个有 context 数据；读取 2 份目录，合并 13 个模型，`Ctx changed: 0`。
- 新旧 `cost --sort context -n 100000 --no-scan` 输出逐字一致，`before-after.diff` 为空。
- 目录读取：启动探针 0.956 / 0.243 / 0.238ms，真实截图运行 1.067ms（另一轮 1.102ms），低于 5ms，不需要额外 mtime 缓存。
- 首批行上屏：改前 460 / 201 / 212ms；改后 357 / 114 / 114ms，最终真实截图运行 229ms。首轮索引读取分别 292 / 267ms，随后约 17–25ms，页缓存和系统状态有明显影响；这些样本只说明未见启动回退，不据此宣称速度提升。

### 截图

归档目录：`tmp/2026-11-03-context-window-catalog/`。

- `screenshots/3-sorted.png`：已打开检查，表格 Ctx `3%` 与 Metadata `3% 31k / 1M · peak 31k` 一致。
- `overflow/screenshots/3-sorted.png`：单独生成的假数据，窗口 100K、最终输入 150K；已打开检查，表格 `150%` 四字符完整显示，Metadata 为 `150% 150k / 100k · peak 150k`。
- 同目录保留 `asm-before`、索引副本、before / after、`windows.txt`、测试与启动日志、验收脚本。没有提交本机目录、账号信息或真实 session 数据。
- 截图前检查 context 占用为 28%，运行 `wake-screen`；没有发送系统按键或操作剪贴板。最后 `pgrep -fl agent-session-manager` 无实例。

## 注意事项

- 把索引复制到临时目录后，默认收藏根也会跟着改变，触发根目录守卫自动重建。第一轮截图因此在扫描中呈现空表；最终截图另用索引副本，并显式指定原 `ASM_CLAUDE_DIR` 和 `ASM_FAVORITES_DIR` 后重拍。真实索引未修改；CLI 对比使用的静态副本也重新核对过输出一致。
- 现有 `make screenshot` 在新二进制首次启动时可能因固定 2.5 秒等待而拿不到窗口，本次首轮遇到并重跑。未扩大本次范围去改截图框架。
- store 的“无变化不发 Loaded”用同步调用 `Worker::scan` 验证，避免等待不存在的事件或依赖超时；完整请求到 UI 的路径另由无界面窗口测试验证。
- 窗口查询全程不读文件；读取只在启动与扫描的后台线程，以及 CLI 每次运行时发生。获取时间或诊断变化本身不刷新整表。

## 遗留问题

Ctx 使用目录当前声明值，没有还原 Claude Code 在会话当时的环境变量、内置上限、账号或 GrowthBook 调整。按计划在 `kb/known-issues.md` 登记，当前本机数据没有目录与内置窗口不一致的样本。未新增其他功能待办。

## 相关文档

- [目录窗口计划](../plans/2026-11-03-context-window-from-catalog.md) — 本次实施依据与完成清单。
- [原费用与 Context 计划](../plans/2026-10-23-cost-context-columns.md) — 补充窗口来源已变更的说明。
- [Claude 数据格式](../docs/claude-session-data-format.md) — 更新模型目录实测结构与读取规则。
- [运行时架构](../docs/app-architecture.md) — 更新目录传播与无 session 变化时的刷新规则。
- [产品 PRD](../docs/product-prd.md) — 更新功能状态与 Ctx 定义。
- [gpui-kit 指南](../docs/gpui-kit-guide.md) — 参考行模型、事件与无界面测试规则。
- [M1 核心实现笔记](../notes/2026-09-17-m1-core-crate.md) — 参考测试数据与 core 约定。
- [已知问题](../known-issues.md) — 更新价格表条目并登记实际窗口限制。
