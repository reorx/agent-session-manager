---
created: 2026-11-16
tags:
  - usage
  - ledger
  - import
  - agent-session-manager
---

# 把已有用量导入真实账本，保留已删 session 的历史

## 协调者摘要（usage-p1-import）

- 完成真实导入与核对；提交为本文件所在的 `Import historical usage into the durable ledger from read-only snapshots` 提交。
- 导入前：真实 `index.sqlite` 是版本 4，129,372 次调用、2,225 个有用量的 session；`index-cli.sqlite` 与 `usage-ledger.sqlite` 均不存在。
- 新调用贡献：备份 129,372，真实索引快照 0；CLI 来源不存在，跳过，没有在真实目录补建。
- 导入后：129,372 次调用、2,225 个有用量的 session、3,361 个 session 元数据、5,510 条小时汇总；56 个已删 session 全部保留。
- 最早用量日：2026-07-13（Asia/Singapore，04:00 分日）；77 天费用合计 $12,965.70402689。
- 逐列调用并集、小时汇总重算、77 天费用、SQLite integrity_check 全过；最大日费用浮点差 $5.684341886080802e-14。
- core 全量：373 通过 / 0 失败 / 6 忽略；新增普通行为测试先红后绿，显式导入测试在演练与真实目标各通过一次。
- 新版 `asm stats` 在额外索引副本与真实账本副本上退出 0；快照索引升级清空后，账本仍保留全部用量。
- 证据：`tmp/2026-11-12-usage-stats/usage-p1-import/`，无 GUI、无截图。
- 自主调整：CLI 来源实际不存在，按空来源处理；CLI 验证用真实索引快照的额外副本替代。
- 未验证：真实 GUI 与生产索引升级（任务明确不做）；没有未通过的完成项。
- 交给协调者：签收后更新计划勾选及共享文档；受保护原始备份仍保留。

## 过程与核对

严格按只读摸底 → 快照 → 测试与入口 → scratch 演练 → 真实导入 → CLI 副本验证的顺序执行。

摸底脚本以 `sqlite3 file:…?mode=ro` 查询真实索引；受保护的 `index-copy.sqlite` 仅通过 `immutable=1` 读取。备份中 `transcript_missing=1` 且存在用量的 56 个 session ID 固定保存到 `missing-sessions.json`。真实索引中这 56 个也全部存在，并没有先前担心的重建丢失。真实索引的 WAL 为 0 字节；只读 SQLite 连接可能更新共享内存协调文件，未执行任何真实索引写入 SQL 或用 `Db` 打开真实索引。

`inventory.json` 保存文件大小、修改时间、账本是否存在及来源统计；`inventory.sql` 保存摸底 SQL。`snapshots.json` 保存实际 backup 命令及哈希。真实索引主文件在 backup 前后 SHA-256 相同；两份生成的快照均为 `5befb3e54fd196c95b99d01bb8ad82a9a220dc3bf3912b80db85883566b1ae8c`。快照完成后不再打开真实索引，所有来源读取均针对 scratch 文件。

现有 `Db::salvage_into_ledger` 虽公开，但必须先有带目标账本的 Db。新增 `Ledger::salvage_into_ledger` 直接复用已有 `read_responses` 和原子的 `write`，无需为导入在真实目录建辅助索引。没有修改解析器、价格表、schema 或现有 Db 恢复路径。

演练目标为 `rehearsal/usage-ledger.sqlite`，实际目标为 `~/Library/Application Support/dev.reorx.AgentSessionManager/usage-ledger.sqlite`。导入顺序是 `backup.sqlite` → `index.sqlite`，来源用 SQLite READ_ONLY 打开。重复调用使用原有主键 upsert、时间取较早值的规则。日志分别是 `rehearsal.log` 和 `live.log`。

`verify.py` 对演练和真实账本执行相同核对：

- 来源主键去重并集与账本调用数相同，双方逐列 EXCEPT 都为空；不仅比较数量。
- 56 个已删 session 同时出现在账本调用表和元数据表。
- 从 responses 重新按全部分组键聚合，与 usage_hourly 双向 EXCEPT 都为空。
- 来源按 UTC+8、本地 04:00 分日，即 `date(ts/1000,'unixepoch','+4 hours')`；账本用 Rust `UsageData::totals` 与当前价格表计费。两份来源的 77 天费用一致，账本逐日相符且无未定价调用。
- SQLite `integrity_check` 为 `ok`。

核对 SQL 和结果分别为 `rehearsal-verify.sql` / `live-verify.sql`、`rehearsal-verification.json` / `live-verification.json`；逐日来源数值在对应的 `*-daily.json`。

最后把真实账本只读 backup 到 `cli-check/usage-ledger.sqlite`，把第 2 步的索引快照再 backup 为 `cli-check/index-cli.sqlite`。设置 ASM_DB 和 ASM_STATE 到该目录，运行 `cargo run -p agent-session-core --bin asm -- stats`。命令成功，版本 4 的副本索引重建为空符合预期（stats 不扫描）；`cli-check-verification.json` 证实账本仍完整。真实账本和真实索引未受此步骤影响。受保护备份、三组 scratch 数据与全部证据均保留。

## 修改的文件

- `crates/core/src/ledger.rs`：可直接向 Ledger 导入来源索引连接的最小入口。
- `crates/core/tests/ledger.rs`：只读旧索引导入、重复导入、来源字节不变的行为测试；显式指定多来源和目标的忽略测试，附导入计数与分日费用输出。
- `kb/docs/data-layer-design.md`：手动历史导入入口与安全流程。
- `kb/plans/2026-11-16-usage-stats/usage-ledger.md`：实际执行结果及缺失 CLI 来源的调整，未修改勾选。
- 本总结。

## 验证

`test-red.log` 记录新增入口不存在时的编译失败；`core-tests.log` 为全量测试输出：373 passed、0 failed、6 ignored。新增忽略测试在 scratch 与真实账本上各显式执行一次通过；`cli-check.log` 记录新版 CLI 退出成功。未改解析器或 CLI 输出行为，因此不要求重跑 parity；未启动 GUI，无测试进程或浏览器需要清理。

## 踩过的坑

- 任务预期的真实 `index-cli.sqlite` 实际不存在；第一次只读查询失败后，改为明确记录 ABSENT，未创建空库冒充来源。
- 独立核对 SQL 的 `A EXCEPT B UNION C` 不能表示 `A EXCEPT (B UNION C)`，需对子查询加括号；已修正后双向逐列比较通过。
- 价格分组的 `LONG_PROMPT` 是项目当前的 100,000，而非 200,000。核对脚本初版使用后者导致小时重算不符，按源码校正后全过；这些核对脚本问题都在 scratch 阶段解决，没有影响账本内容。

## 交给协调者

- 更新第 1 期步骤 7 的勾选、AGENTS.md 状态与账本规则；按 brief 分工，本任务未改共享文件。
- 可依据本次证据关闭“重建索引后，记录文件已被删掉的 session 的用量永久丢失”已知问题；此次找回/保护范围为账本上线前可读来源中的全部 129,372 次调用与 56 个已删 session。
- `index-copy.sqlite` 及 WAL/SHM 均未删除或移动，是否清理仍由协调者决定。
- 工作区没有出现非本任务的未提交改动。无范围外代码问题、无待修复缺陷。

## 相关文档

- [用量统计总计划](../plans/2026-11-16-usage-stats/index.md) — 共用口径依据。
- [用量账本设计](../plans/2026-11-16-usage-stats/usage-ledger.md) — 步骤 7 的执行与实测更新。
- [数据层设计](../docs/data-layer-design.md) — 手动导入方法更新。
- [M1 core 笔记](../notes/2026-09-17-m1-core-crate.md) — core 修改规则参考。
