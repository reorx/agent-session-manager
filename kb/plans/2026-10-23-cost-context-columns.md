---
created: 2026-10-23
tags:
  - cost
  - context-window
  - pricing
  - table
  - date-picker
  - agent-session-manager
  - plan
---

# 按项目看花费：Cost 列、Context 列、自定义时间范围

## PRD

### 背景

用户想知道某个项目在一段时间里花了多少钱、哪几个 session 最贵，也想找出结束时 context 占用特别高的 session，以便优化提示和工作方式。现在表格里看不到费用，详情里的费用来自 Claude Code 自己写的记录，很多 session 没有这条记录或者记成 0；时间过滤只有「今天 / 3 / 7 / 30 天 / 不限」几个预设，选不了「上个月」或任意一段日期。

### 目标

选定一个项目和一段日期，表格底部直接显示这些 session 的费用合计；按费用或 context 占用排序就能找到最贵、最「撑」的 session。

### 功能与交互

- **Cost 列**：放在 Prompts 后面，显示该 session 整个生命周期的费用（如 `$3.42`），右对齐。费用由本应用按 token 用量和官方价格自己算，包含该 session 的 subagent 的花费。算不出来（没有任何用量记录，或模型不在价格表里）显示 `—`。可以点表头排序，没有费用的视为最低（降序时排在最后，和 Size 等列的空值一致）。
- **Ctx 列**：紧跟 Cost，显示 session 最后一次调用时 context 的占用比例（如 `41%`，不足 1% 显示 `<1%`，无数据 `—`）。可排序，无数据的视为最低。
- **费用合计**：状态栏左侧从 `42 / 5488 sessions` 变成 `42 / 5488 sessions · $123.45`，合计的是当前表格里显示的行。搜索中只合计命中的行。
- **时间范围**：Updated 下拉在现有预设后加「This month」「Last month」，分隔线后加「Custom range…」。选 Custom 后，Updated 按钮右边出现一个日期区间选择器：一次显示两个月，左侧有「本月 / 上月 / 近 7 天 / 近 30 天」快捷项，第一次打开时默认是本月 1 日到今天。选完区间表格立刻过滤，键盘焦点回到表格。选回别的预设时选择器消失。区间和预设都在重启后保留。
- **时间过滤的含义不变**：一个 session 是否落在范围内，看它的 Updated 时间；它的 Cost 仍是整个 session 的费用，不按日期切分。
- **详情 Metadata**：Cost 显示自算的值，旁边注明 Claude Code 自己记的数（`CC: $x.xx`）便于对照；新增 Tokens（输入 / 输出 / 缓存读 / 缓存写）和 Context（最终占用 / 窗口大小 · 峰值）。有算不了价的模型时列出模型名。
- **命令行**：`asm cost` 按项目和日期列出 session 的费用、context 占用并给出合计；`--compare` 统计自算费用和 Claude Code 记录的偏差分布。

### 不做的事

- 不按天切分一个跨越多天的 session 的费用。
- 不做费用图表、预算或提醒。
- 不统计 transcript 里没有记录的调用（标题生成、分类器等）。

### 验收标准

- 表格有 Cost、Ctx 两列，可排序，Preview 列没有被挤坏。
- 选项目 + 区间后，状态栏显示合计；改过滤或搜索，合计跟着变。
- 选「Last month」只剩上个月里更新过的 session；自定义区间同理，结束日当天的 session 也包含在内。
- 设置自定义区间后，表格上的快捷键仍然可用。
- 重启后区间、预设、按 Cost 的排序都恢复。
- 在本机数据上，≥90% 的 session 自算费用与 Claude Code 记录偏差在 10% 以内，全部合计相差不到 1%。

## Tech Design

### 现状

- 表格 7 列（`crates/app/src/table.rs`），列宽常量和 `FIXED_COLS_W` 在文件顶部，Preview 吃剩下的宽度（`main.rs::sync_preview_width`）。状态栏在 `main.rs::render_status_bar`。
- `SessionSummary.total_cost_usd` 来自 `cost-state` 记录（`claude/parse.rs`），只在详情 Metadata 里显示（`SessionRow::cost_fmt`）。
- 解析器只读 assistant 记录的 `input_tokens` / `output_tokens`，而且只给带文本的记录建 `MessageRow`；缓存 token 不读。subagent transcript（`<session>/subagents/*.jsonl`）只计数，不解析。
- 时间过滤：`model.rs::RangePreset` 五个预设，`since_ms()` 只给下界；`Filter.since_ms`。

### 实测依据（本机，2026-10-23）

- assistant 记录 117,536 条，对应 53,525 次 API 响应。一次响应按内容块拆成多条记录，`message.id` 相同，usage 完全相同，**必须按 `message.id` 去重**，否则 token 多算约 2.2 倍。主 transcript 里没有 `isSidechain` 的记录。
- subagent transcript 257 份、196MB、5,545 次响应；cache read 6.16 亿 token、cache write 3,157 万 token，折合几百美元。
- `cost-state` 出现在 1,636 份文件里；670 份 `totalCostUSD = 0`，更早的 session 没有这条记录。抽样核对：`cost-state.modelUsage.costUSD` 与「1h 缓存写入按 2 倍输入价」的算法精确一致（Fable 5.1 $14.1829，Opus 5 $5.8166）。
- 966 个 `cost-state` 非 0 的 session：自算误差 1% 以内 592 个，1–10% 311 个，偏低 >10% 46 个，偏高 >10% 17 个；合计自算 $7,103 对 $7,096。偏低推测来自 transcript 之外的调用，未验证。
- `message.model` 从不带 `[1m]`；有 session 超过 200K（最高 827K），所以窗口按模型查表：Haiku 4.5 为 200K，其余 1M。1,233 个 session 里 379 个结束时超过 200K。
- `speed: "fast"` 422 条；`web_search_requests` 全为 0；`service_tier` 全为 standard。

价格（$/MTok，核对来源和日期写在 `pricing.rs` 注释里）：

| 模型 | 输入 | 输出 | 缓存读 | 窗口 | 备注 |
|---|---|---|---|---|---|
| opus-5-5 | 4 | 20 | 0.20 | 1M | fast ×2 |
| opus-5 | 5 | 25 | 0.50 | 1M | fast ×2 |
| opus-4-8 / 4-7 / 4-6 | 5 | 25 | 0.50 | 1M | |
| fable-5-1（mythos-5-1） | 10 | 50 | 0.25 | 1M | |
| fable-5（mythos-5） | 10 | 50 | 1.00 | 1M | |
| sonnet-5-5 | 2 | 10 | 0.10 | 1M | 缓存读是输入价的 5% |
| sonnet-5 | 2 | 10 | 0.20 | 1M | |
| sonnet-4-6 | 3 | 15 | 0.30 | 1M | |
| haiku-5-5 | 0.10 | 0.50 | 0.01 | 1M | prompt 超过 100K 时整体 ×5 |
| haiku-4-5 | 1 | 5 | 0.10 | 200K | |
| opus-4-5 / opus-4-1 / opus-4、sonnet-4-5 / sonnet-4、haiku-3-5 | 见 `pricing.rs` | | | 200K | 本机没有，按官方表补齐 |

缓存写入：5 分钟 TTL 按输入价 ×1.25，1 小时 TTL ×2；没有 `cache_creation` 细分的旧记录按 5 分钟算。web search 每次 $0.01。fast mode 的价格官方页写明是 Opus 5.5 / 5 / 4.8 输入输出各 ×2，缓存倍率叠加在 fast 价格上，所以整体 ×2 是官方口径，不是推定。2026-10-23 用 firecrawl 抓官方价格页核对，唯一的出入是 Sonnet 5.5 的缓存读（方案初稿写成了 0.20）。

### 方案

core 在解析时为每次 API 响应产出一条 `UsageRow`（按 `message.id` 去重、已计价），写进新表 `api_usage`，主键 `(session_id, agent, msg_id)`，`INSERT OR REPLACE` 让被切在两次增量扫描之间的同一响应只算一次；每次写入后用一条 SQL 把汇总刷回 `sessions` 的新列。subagent 文件作为独立的扫描单元进入同一套增量机制，只产出 usage，计入父 session。app 只消费 `SessionSummary` 上的新字段。

关键决策：

- **自算费用，`cost-state` 只作对照**：覆盖率低（大量 0 和缺失），不含 subagent 以外的拆分；自算在合计上与它相差 0.1%。
- **用量按行入库而不是在 `SessionDelta` 里累加**：增量扫描可能把一次响应的几条记录切成两半，只有按 `msg_id` 去重的表才能保证「全量 == 增量」。备选「delta 里存已见 id 集合」需要跨扫描持久化，等于重新发明这张表。
- **context 只取主线最后一次调用**：`context_tokens` / `context_model` 是 last 类，`context_peak` 是新的 max 类（幂等，增量与全量一致）。
- **价格写死在代码里**：价格调整时提升 `SCHEMA_VERSION` 触发整库重建（约 4 秒），比做迁移简单。
- **先写 subagent 的用量，再写 session 行**（初稿是反过来）：反过来的话，一个 subagent 有变化的 session 会先带着旧费用发一次 `Session`，再带着新费用发一次，流出的行和索引不一致（有测试断言流出的行等于索引里的行）。`api_usage` 没有外键，所以父行还没写也能先插用量。
- **subagent 文件消失时看父 session 的记录文件还在不在**：在，说明 subagent 是单独删的，删掉它的用量；不在，说明整个 session 被清理，用量保留。初稿只写了"删用量"，那样被清理的 session 会丢掉 subagent 那部分费用，和"主文件消失保留用量"矛盾。
- **subagent 用只读用量的解析模式**（`TranscriptParser::usage_only()`）：跳过 user 和元数据记录、不建 `MessageRow`，多出的 196MB 只多花约 0.2s。
- **日期区间用 gpui-component 自带的 `DatePicker` 区间模式**：API 以本地 `gpui-component-0.6.1/src/time/date_picker.rs` 为准；区间模式只在两端都选好后发 `Change`，快捷项直接发 `Change`。

### 数据变动

- 新表 `api_usage(session_id, agent TEXT NOT NULL DEFAULT '', msg_id, model, ts, input, cache_write_5m, cache_write_1h, cache_read, output, web_search, speed, cost_usd, PRIMARY KEY(session_id, agent, msg_id)) WITHOUT ROWID`。`agent` 为 `''` 表示主 transcript，否则是 subagent 文件名 stem。
- `sessions` 加列：`cost_usd REAL`、`tokens_in`、`tokens_out`、`tokens_cache_read`、`tokens_cache_write`、`context_tokens`、`context_peak`、`context_model`、`unpriced_models`（JSON 数组）。
- `index_state` 加 `agent` 列。
- `SCHEMA_VERSION` 1 → 2，旧索引整库重建。
- `state.json`：`range` 新增 `ThisMonth` / `LastMonth` / `Custom`；新增 `custom_range`（`["YYYY-MM-DD", "YYYY-MM-DD"]`）。旧文件缺字段时取默认值。

### 逻辑变动

**core**

- `pricing.rs`（新）：`normalize(model)` 去 `[1m]` 和 `-YYYYMMDD`；`price(model) -> Option<ModelPrice>`；`cost(model, &UsageRow) -> Option<f64>`；`context_window(model) -> Option<u64>`。
- `claude/parse.rs`：`RawMessage` 加 `id`，`RawRecord` 加 `requestId`；`RawUsage` 补缓存、`cache_creation{ephemeral_5m_input_tokens, ephemeral_1h_input_tokens}`、`server_tool_use{web_search_requests}`、`speed`。`on_assistant` 在解析 content 之前产出 `UsageRow`（跳过 `<synthetic>`；去重键 `message.id` → `requestId` → `uuid`；同一次 feed 内去重）。`Feed` / `ParsedTranscript` 加 `usage`。`SessionDelta` 加 `context_tokens`、`context_model`（last，只看非 sidechain）和 `context_peak`（max）。`parse_transcript()` 在内存里按同样规则汇总到 summary。
- `model.rs`：`UsageRow`（`fast: bool` 代替初稿的 `speed` 字符串，计价只关心是不是 fast）；`SessionSummary` 加 `cost_usd`、`tokens_*`、`context_*`、`unpriced_models`，以及 `set_usage`（内存里按 `msg_id` 去重汇总，`parse_transcript` 和测试用）、`context_ratio`。`models` 不再包含 `<synthetic>`。
- `index/db.rs`：`insert_usage`、`delete_usage(session_id, agent: Option<&str>)`、`refresh_session_usage(session_id)`；`SESSION_COLUMNS` / `upsert_session` / `row_to_session` 同步新列（usage 汇总列不由 `upsert_session` 写，只由 `refresh_session_usage` 写）；`clear_all` 清 `api_usage`；`index_state` 读写 `agent`。
- `claude/discover.rs`：`TranscriptFile` 加 `agent`（session 自己的文件为空串）；新增 `discover_subagent_transcripts(claude)`，`session_id` 填父 session；`discover_transcripts` 的结果不变（`transcript.rs`、`regex.rs` 在用）。
- `ScanReport` 加 `subagents_seen` / `subagents_parsed`；`files_*` 计数和进度条只数 session 的记录文件，界面和已有测试的口径不变。
- `index/indexer.rs`：主文件和 subagent 文件一起 `plan_for`、一起 rayon 解析；写库先主文件后 subagent。主文件非 Append 时先 `delete_usage(id, Some(""))`；subagent 只 `insert_usage` + `refresh_session_usage(父)` + `emit_session`，父行不存在时跳过。`mark_missing`：subagent 消失 → 删它的 usage、刷新父 session、删 index_state，不把父标成 missing；主文件消失保留 usage。
- `bin/asm.rs`：`cost` 子命令（`--project`、`--since`、`--until`、`--sort cost|context`、`--limit`、`--compare`）。`list` / `session` 输出不变。`ListFilter` 加 `until_ms`；`format.rs` 加 `fmt_usd` / `fmt_tokens`，CLI 和界面共用。

**app**

- `model.rs`：`SessionRow` 加 `cost: Option<f64>`、`cost_fmt`（自算）、`cc_cost_fmt`、`context_pct`、`context_fmt`、`context_detail`、`tokens_fmt`、`unpriced`。`RangePreset` 加 `ThisMonth` / `LastMonth` / `Custom`，`since_ms()` 改为 `bounds(custom)`（可测的 `bounds_on(today, custom)`），上界取结束日次日本地零点（不含）；Custom 没有选过区间时按「本月 1 日到今天」。`Filter` 加 `until_ms`。
- `table.rs`：`Cost`（62px）、`Ctx`（44px）列在 Prompts 后；`FIXED_COLS_W` 加上两列，`PREVIEW_MIN_W` 从 240 降到 200（默认窗口下 Preview 约 290px，搜索时多出 Matches 列也不出现横向滚动）；`compare` 加 `cost` / `context`（`total_cmp`，`None` 最低）；`recompute()` 缓存 `shown_cost`。
- autoplay 的第二步从按 Prompts 排序改为按 Cost 降序，`make screenshot` 的第 3 张图顺带验收新列。
- `main.rs`：状态栏左侧加合计；`DatePickerState` 实体与订阅；视图状态存取 `custom_range`。
- `toolbar.rs`：Updated 菜单加项和分隔线；Custom 时在按钮右侧渲染 `DatePicker`（2 个月、4 个快捷项）。`Change` 时更新过滤并 `focus_table`。
- `settings.rs`：`custom_range: Option<(NaiveDate, NaiveDate)>`。app 的 chrono 打开了 `serde` feature，`NaiveDate` 在 JSON 里就是 `"YYYY-MM-DD"`，比手写字符串转换少一层；只有用户选过区间才写入，避免把某天的「本月」默认值存成固定日期。
- `detail.rs`：Metadata 的 Cost / Tokens / Context / 未计价模型。

### 实施步骤

- [x] 0. 本 plan 落盘；firecrawl 抓官方 pricing 页核对价格
- [x] 1. `pricing.rs` + 测试
- [x] 2. fixture 生成脚本加 usage、拆分响应、subagent usage；parser 测试；实现 parser 改动
- [x] 3. schema / db；index 测试；实现 db
- [x] 4. indexer + discover；index 测试通过
- [x] 5. `asm cost`；跑 `--compare`
- [x] 6. app 表格两列与合计（含测试）
- [x] 7. app 自定义时间范围（含 settings / keyboard 测试）
- [x] 8. 详情 Metadata
- [x] 9. 文档、known-issues、session 笔记，一次性 commit

### 测试与验证

- `pricing`：两份真实 `cost-state.modelUsage` 基准（Fable 5.1 $14.18291425、Opus 5 $5.816622）；Haiku 5.5 的 100K 分档；fast ×2；`[1m]` 和日期后缀归一化；未知模型 `None`；窗口查表。
- `parse_transcript.rs`：拆成 3 条记录的响应只算一次；`<synthetic>` 跳过；`context_tokens` 取最后一次、`context_peak` 取最大；逐行切开分批 feed 的 `apply` 结果与全量相同（含 context 字段）。
- `index.rs`：同一 `message.id` 被切在两次扫描之间时费用等于全量；subagent usage 计入父 session；subagent 追加后父 session 费用更新并重新发出；subagent 删除后费用下降且父不被标 missing；主文件 Rebuild 后 subagent usage 还在；旧版本索引整库重建。
- app：cost / context 格式化；`RangePreset` 本月、上月、自定义的上下界；`Filter.until_ms` 边界；按 cost / context 降序排序、`None` 最后；`shown_cost` 跟随过滤和搜索；`custom_range` 在 `state.json` 往返；旧 `state.json` 可加载；设置自定义区间后表格快捷键可用。
- 实机：`make test`、`make parity`；`tmp/` 索引副本冷启动，记录扫描耗时（之前 3.6–4s）、首批行上屏耗时和 DB 大小；`make asm ARGS="cost --compare"`；截图（预设「项目 + 自定义区间 + 按 Cost 降序」「按 Ctx 降序」），校对 `scripts/screenshot-clicks.sh` 坐标，补一张 DatePicker 弹层截图，归档到 `tmp/2026-10-23-cost-context/`。

### 风险与待定

- 自算费用不含 transcript 之外的调用，单个 session 可能偏低 10% 以上（已登记 known-issues）。
- 实测结果（2026-10-23）：973 个 `cost-state` 非 0 的 session，误差 1% 以内 61.3%、1–10% 34.9%、偏低 >10% 2.3%、偏高 >10% 1.5%；合计自算 $7,140 对 $7,217（−1.07%），略超「相差不到 1%」的预期。偏高的大户是被 resume 过的 session，`cost-state` 只记了最后一次运行；方案初稿里 +0.1% 的合计是在 `claude-opus-5[1m]` 的调用没计价的情况下算的，口径不同。
- 冷启动（页缓存热）从 3.45–3.53s 变成 3.62–3.77s，索引 91.5MB → 100.2MB（`api_usage` 8.1MB，59,363 次响应）。
- `DatePicker` 的弹层在工具栏里的定位和尺寸未验证；`Change` 之后焦点是否能交还表格需在键盘测试里确认。
