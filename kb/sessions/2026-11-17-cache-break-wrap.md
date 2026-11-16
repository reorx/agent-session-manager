---
created: 2026-11-17
tags:
  - cache
  - documentation
  - agent-session-manager
---

# 缓存准确性文档收尾与已知问题关闭

## 协调者摘要（cbreak-tracker-wrap）

- `kb/plans/2026-11-16-cache-break-accuracy.md`：步骤 2–6 打勾，各自补完成日期、提交和总结链接；Tech Design 同步 RequestFacts 的 Option、beta 集合规范化、字段级降级、冻结审计入口与热缓存耗时结论。旧现状明确标成实施前基线。步骤 7、8 保持未勾。
- `AGENTS.md`：缓存入口改为 CacheTracker，说明祖先比较、请求事实与分段时长；补账本解析升级和未公开字段降级规则，更新 Documentation 的入口。没有改「当前状态」。
- `kb/known-issues.md`：删除本次已修复的零外层用量和 rewind 两条；未改写其余条目，未新增问题。
- `kb/docs/claude-session-data-format.md`：补 R1 的字段级降级及正文、工具和导出保留规则。
- `kb/docs/product-prd.md`：详情状态表的旧四原因描述改为当前规则，明确逻辑与真实数据审计通过、截图待完成；具体原因仍由原有「缓存重建说明」一处描述。
- 本总结记录关闭依据与交接。仅改上述六份文档，一个提交；不改代码，不运行 cargo / make，不启动或干预 app，不碰截图代理的目录。
- 交给协调者：审本次 diff；签收截图代理的步骤 7 后，再决定勾选步骤 7、8，并更新 AGENTS.md「当前状态」及产品状态表中的截图状态。

## 概要

基于已签收的 iterations（e0d9f82）、tracker（b408f4a）、真实数据审计（12ad8f0）和 R1（51ec13f），将现状写回共享文档。对照生产源码核实 RequestFacts、Reader 请求表、cache_field 与审计测试的环境变量；没有重跑构建或验收。

## 已解决的已知问题

- **费用与缓存失效：顶层 usage 全为 0 的响应漏算费用，回合卡误报一次失效、漏报下一次**：非空 iterations 全项计费，缓存比较取最后一个 message 项。四次误报消失，四次后续重建补出为 compact；相同输入费用增加 $0.3735984，真实账本副本的四个响应、小时汇总和用量日增量一致。原账本 146246 个响应主键全部保留。依据为 iter 和 audit 总结。
- **回合卡：`/rewind` 之后的第一次调用被标成缓存失效**：按 parentUuid 接续的祖先调用比较。原四次不明变成三次不报、一次换模型；另四次原空闲变成三次空闲、一次换模型。3398 个冻结前缀、133186 次调用、842 条重建与独立参考 TSV 逐字节一致，R1 后复跑仍为零差异。依据为 tracker、audit 与 R1 总结。

记录文件之外的 API 调用漏计、冷缓存打开耗时等已有问题不属于本次修复，保留原条目。未发现需要新增到 known-issues 的使用或稳定性问题。next-up 的两项分别等待下次发版和下次布局修改，本次均未触发。

## 核对与边界

文档检查包括 diff 空白检查、修改范围、步骤 2–6／7–8 的勾选状态、AGENTS.md 当前状态逐字不变、相对链接目标存在，以及旧入口在 AGENTS.md / kb/docs 中不再残留。没有运行 cargo 或 make；引用的测试数量来自已签收记录：R1 为 673 passed / 11 ignored，冻结审计为 842 条零差异。

parity 保留实际结果：两轮退出码都是 1，剩余差异仅在持续写入会话的大小、更新时间和结果块顺序，按 brief 决定 8 签收。热缓存读取中位数增量 0.529–0.748ms，不能据此宣称冷缓存、UI 首帧或峰值内存已验证。

任务目录沿用 `tmp/2026-11-16-cache-break-accuracy/`，已有证据在 `cbreak-tracker/audit/` 与 `cbreak-tracker/r1/`；本轮没有生成临时产物，没有访问或改动 `screenshots/`、`cbreak-shots/`。

## 相关文档

- [缓存准确性计划](../plans/2026-11-16-cache-break-accuracy.md) — 更新步骤和实现口径。
- [已知问题](../known-issues.md) — 删除上列两条已解决问题。
- [数据格式](../docs/claude-session-data-format.md) — 补齐字段级降级现状。
- [产品 PRD](../docs/product-prd.md) — 同步详情状态表。
- [iter 总结](2026-11-17-cache-break-iter.md) — 用量与账本升级依据。
- [tracker 与 R1 总结](2026-11-17-cache-break-tracker.md) — 判定、Reader、文案和降级依据。
- [audit 总结](2026-11-17-cache-break-audit.md) — 真实数据、账本、性能与 parity 证据。
