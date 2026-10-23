---
created: 2026-10-23
tags:
  - cost
  - context-window
  - pricing
  - subagent
  - date-picker
  - agent-session-manager
---

# 按项目看花费：Cost 列、Ctx 列与自定义时间范围

## 概要

用户想选一个项目和一段时间，看这段时间的总花费、找出最贵和 context 占用最高的 session。按已确认的方案实现：core 为每次 API 响应产出按 `message.id` 去重、按官方价格计价的用量行，存进新表 `api_usage`（subagent 的记录文件进同一套增量扫描，只取用量，计入父 session），汇总回 `sessions` 的费用 / token / context 列；`asm cost [--compare]` 在命令行上列费用、做与 `cost-state` 的比对。界面加了 Cost、Ctx 两列（可排序）、状态栏费用合计、Updated 下拉的 This month / Last month / Custom range…（gpui-component 的 `DatePicker` 区间模式，两个月 + 4 个快捷项），详情 Metadata 显示自算费用与 `CC:` 对照、Tokens、Context。先写行为测试再实现：core 新增价格、解析、索引、CLI 测试，app 新增格式化、区间上下界、排序、合计、`state.json` 往返和日期选择器的焦点测试，全部通过。真实数据上 96.2% 的 session 自算费用与 Claude Code 自己记的相差不到 10%，合计 −1.07%。实机验收交给 Herdr 新 tab 里的 agent（本 session context 已到 40%），截图全部通过，期间发现一个改动前就存在的 autoplay 卡死问题。

## 修改的文件

- `crates/core/src/pricing.rs`（新）：价格表（2026-10-23 对照官方价格页核对）、`normalize` / `price` / `cost` / `context_window`
- `crates/core/src/model.rs`：`UsageRow`；`SessionSummary` 加费用、token、context、未计价模型字段，`set_usage`、`context_ratio`
- `crates/core/src/claude/parse.rs`：读完整 `usage`、`message.id`、`requestId`；每次响应一行用量（feed 内去重）；`SessionDelta` 加 context（last）和 peak（max）；`<synthetic>` 不计价也不进模型列表；`TranscriptParser::usage_only()` 给 subagent 用
- `crates/core/src/claude/discover.rs`：`TranscriptFile.agent`，`discover_subagent_transcripts`
- `crates/core/src/index/schema.sql`、`db.rs`：`api_usage` 表、`sessions` 新列、`index_state.agent`；`insert_usage` / `delete_usage` / `refresh_session_usage` / `usage`；`ListFilter.until_ms`；`SCHEMA_VERSION` 2
- `crates/core/src/index/indexer.rs`：subagent 文件一起规划和解析；先写 subagent 用量再写 session 行；消失的 subagent 按父记录文件是否还在决定删不删用量；`ScanReport` 加 `subagents_seen` / `subagents_parsed`
- `crates/core/src/format.rs`：`fmt_usd`、`fmt_tokens`
- `crates/core/src/bin/asm.rs`：`cost` 子命令（`--project`、`--since`、`--until`、`--sort`、`-n`、`--compare`）
- `crates/core/tests/`：新增 `pricing.rs`；`parse_transcript.rs`、`index.rs`、`cli.rs` 加用量、context、subagent、分批扫描的测试；`fixtures/gen_fixtures.py` 加完整 usage、拆成 3 条记录的响应、fast mode、`<synthetic>`、subagent 用量并重新生成 fixture
- `crates/app/src/model.rs`：`SessionRow` 的费用 / context / token 字段；`RangePreset` 加 ThisMonth / LastMonth / Custom 和 `bounds` / `bounds_on`；`Filter.until_ms`
- `crates/app/src/table.rs`：Cost（62px）、Ctx（44px）列与排序，`shown_cost`，`PREVIEW_MIN_W` 200；数字列真正右对齐（验收 agent 改的：`text_right` 之前只是列上的标记，单元格和表头没有照做）
- `crates/app/src/main.rs`、`toolbar.rs`、`settings.rs`、`detail.rs`：日期选择器实体与订阅、Updated 菜单、状态栏合计、`custom_range` 持久化、Metadata 新行；autoplay 第二步改按 Cost 排序
- `crates/app/src/keyboard_tests.rs`：选完自定义区间后表格过滤正确、焦点回到表格、快捷键可用
- `crates/app/Cargo.toml`：chrono 打开 `serde` feature
- `scripts/screenshot-clicks.sh`（验收 agent 改的）：先激活窗口、点击前先悬停；Updated 步骤挪到选项目之前；末尾加自定义区间的 4 步，共 16 张
- 文档：`AGENTS.md`、`kb/docs/product-prd.md`、`kb/docs/claude-session-data-format.md`（新增 §2.4 用量与费用，刷新规模数字）、`kb/docs/app-architecture.md`、`kb/docs/data-layer-design.md`、`kb/known-issues.md`、`kb/plans/2026-10-23-cost-context-columns.md`（新）

## 注意事项

- **一次响应多条记录**：Claude Code 按内容块把一次响应写成多条 assistant 记录，usage 相同，不去重 token 多算约 2.2 倍。增量扫描会把它们切在两次扫描之间，所以去重只能靠 `api_usage` 的主键，不能在 `SessionDelta` 里累加。
- **写入顺序决定流出的行是否准确**：先写 subagent 用量、再写 session 行，每个 session 一次扫描只发一次 `Session`，而且费用已含 subagent。有测试断言流出的行等于索引里的行。
- **价格表改了要提升 `SCHEMA_VERSION`**：费用在索引时算好存表。官方价格页核对出方案初稿的一处错误：Sonnet 5.5 的缓存读是 $0.10，不是 $0.20；fast mode ×2（缓存倍率叠加其上）是官方口径。
- **`cost-state` 不适合当基准**：被 resume 过的 session 常常只记了最后一次运行（一个例子里只覆盖了 32M cache read 中的 3.1M），所以 `--compare` 里偏高的大户多半是它少记了。方案初稿 +0.1% 的合计是在 `claude-opus-5[1m]` 的调用没计价时算出来的，口径不同。
- **`DatePicker` 的行为**（本地源码核实）：区间模式只有两端都选好才发 `Change`，快捷项直接发 `Change`；选择后它会把焦点交给自己，所以订阅里要 `focus_table`，否则表格快捷键失效。
- **截图脚本的两个坑**（验收 agent 发现）：脚本启动的窗口不在前台，第一次点击只激活窗口；指针跳过去马上点，GPUI 按上一帧的悬停状态判定点中什么，可能落空。现在先点标题栏、每次点击前先悬停 200ms。
- **`Iterator::sum` 对空的 f64 迭代器返回 `-0.0`**，会打印成 `$-0.00`，`asm cost` 改用 `fold(0.0, ..)`。
- 实测：冷索引（页缓存热）3.45–3.53s → 3.62–3.77s，索引 91.5MB → 100.2MB（`api_usage` 8.1MB，59,363 次响应，其中 5,552 次来自 subagent）。验收 agent 测的热启动首批行 150–189ms；冷启动 `first rows in 3211ms` 是页缓存冷的数字（扫描 6.81s、解析 3.1s），和以前页缓存热时的约 1.0s 不可比，它准备做的热缓存对比被中断，没有数。
- `make parity` 失败 2 组，与改动前的二进制逐字相同，不是本次引入的。

## 遗留问题

- autoplay 第二步在真实数据上让主线程空转，`make screenshot` 的第 3 张停在第一步；改动前的 HEAD 同样复现。已登记 known-issues。验收 agent 在自己的 scratchpad 里写了复现测试和修法的草稿，调查被中断，未合入。
- 自算费用不含 transcript 之外的调用，22 个 session 偏低超过 10%；价格表写死，新模型的调用不计价且表格不提示。均已登记 known-issues。
- parity 的两类差异补进了已有条目。
- 冷启动首批行上屏的热缓存对比没有做；需要时在 `tmp/` 的索引副本上分别用新旧二进制各启动一次。

## 已解决的已知问题

- **文档：数据格式调研里的本机规模数字已过时**：`kb/docs/claude-session-data-format.md` 刷新为 2026-10-23 的统计（1,942 份主 transcript 2.9GB、257 份 subagent 196MB、最大 70MB），并新增用量与费用一节。

## 相关文档

- [费用 / context 列与自定义时间范围方案](../plans/2026-10-23-cost-context-columns.md) — 本次新建并按此实现
- [产品 PRD](../docs/product-prd.md) — 更新表格列、时间过滤、状态栏与状态表
- [Claude Code Session 数据格式调研](../docs/claude-session-data-format.md) — 新增 §2.4，刷新规模数字
- [运行时架构](../docs/app-architecture.md) — 新增 subagent 记录文件的扫描规则、焦点与视图状态
- [数据层设计](../docs/data-layer-design.md) — 顶部补充 `api_usage` 的说明
- [gpui-kit 开发指南](../docs/gpui-kit-guide.md) — 改界面前参考
