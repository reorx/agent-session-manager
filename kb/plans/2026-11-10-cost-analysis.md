---
created: 2026-11-10
tags:
  - cost
  - pricing
  - fable
  - detail-panel
  - table
  - agent-session-manager
  - plan
---

# 成本分析：换一个模型，这些 session 要花多少钱

## PRD

### 背景

Fable 5.2 发布（输入 $8 / 输出 $40 / 缓存读 $0.20 每百万 token），价格是 Opus 5.5（$4 / $20 / $0.20）的两倍，只有缓存读同价。用户日常主要用 Opus 5.5，想知道：如果这些 session 换成 Fable 5.2（或者 Sonnet 等其他模型）会多花多少钱，哪类 session（context 撑得满的、输出多的）涨得最多。现在应用只显示按实际模型算出的费用，看不到输入 / 输出 token 的量，也不能按模型筛 session，更没法换个模型重算。

### 目标

在应用里过滤出某个模型的 session，按 context 占用或 token 量排序；点开任一 session 就能看到它的单价、输入 / 输出费用和总价，并且切换成别的模型看预计花费与差额；状态栏同时给出当前表格所有行换模型后的合计。

### 功能与交互

- **价格表加 Fable 5.2**（及同价的 Mythos 5.2）：输入 $8、输出 $40、缓存读 $0.20、5 分钟缓存写 $10、1 小时缓存写 $16，fast mode 价格翻倍。
- **主模型**：一个 session 里可能有多个模型（中途切换、Haiku 子代理）。花费最高的那个是它的主模型。
- **Model 过滤**：工具栏 Kind 后面加「Model」下拉，第一项「All」，下面按 session 数从多到少列出表格里出现过的主模型（显示名如「Opus 5.5」，右侧是 session 数）。选中后只显示主模型是它的 session。重启后保留。
- **In / Out 列**：表格在 Ctx 后面加两列，In 是全部输入 token（未缓存输入 + 缓存写 + 缓存读，反映每次请求实际送进模型的量），Out 是输出 token，格式如 `13.1M`、`120k`，可点表头排序，无数据视为最低。
- **详情 Cost tab**：详情的 Metadata / Conversation 后面加第三个 tab「Cost」。
  - 顶部「Model」：主模型的显示名、模型 id、调用次数，下面一行是它的单价（输入、输出、5 分钟 / 1 小时缓存写、缓存读，每百万 token）。
  - 「Compare with」：一个下拉，列出可比较的模型（当前一代 Fable 5.2 / Opus 5.5 / Sonnet 5.5 / Haiku 5.5 在前，分隔线后是较早的模型），每项带输入 / 输出单价；下面一行是所选模型的单价。默认 Fable 5.2；选过之后记住，切换 session 不变，重启后保留。
  - 费用表：行是 Input（未缓存）、Cache write 5m、Cache write 1h、Cache read、Input total、Output、（有才显示的）Web search 与 Other models、Total；列是 Tokens、主模型的费用、所选模型的费用、变化百分比。token 为 0 的缓存行不显示。
  - 只换主模型的调用；其他模型的调用（如 Haiku 子代理）按原价计入两边的总价，在 Other models 行列出。
  - 有 fast mode 调用时注明按 fast 价计（目标模型没有 fast mode 则按标准价）；目标是 Haiku 5.5 且有 prompt 超过 10 万 token 的调用时注明按 5 倍计。
  - 底部一行说明：预测按相同 token 重新计价，换模型后对话本身会不同，只能作为参考。
  - 空状态：没有用量记录的 session（History 行、早期记录）显示「No API usage recorded for this session」；主模型不在价格表里时实际费用显示 `—`。
- **状态栏合计**：Cost tab 打开时，状态栏在 `N / M sessions · $X` 后接 `· Fable 5.2 $Y (+Z%)`，按所选模型重算当前表格所有行（只换各自的主模型），过滤、搜索、切换比较模型时跟着变。
- **命令行**：`asm cost --as MODEL` 在每行加上换模型后的费用和变化，合计处给出按 token 种类拆分的对比；`--model MODEL` 只列主模型是它的 session。模型名可省略 `claude-` 前缀。

### 不做的事

- 不预测换模型后 token 数量或轮数的变化，只按同样的 token 重新计价。
- 不做「换模型后费用」的表格列和按它排序。
- 不把子代理的模型一并替换。
- 不做 Batch、Priority Tier、US-only inference 等计价方式。

### 验收标准

- 价格表里有 Fable 5.2 / Mythos 5.2，`asm cost --model opus-5-5 --as fable-5-2` 给出本机 Opus 5.5 session 的合计对比。
- 工具栏有 Model 下拉，选 Opus 5.5 后只剩主模型是 Opus 5.5 的 session；重启后仍然生效。
- 表格有 In / Out 两列，可排序；Ctx 降序仍可用。
- 点一个 session，切到 Cost tab，能看到主模型、单价、输入 / 输出费用和总价；切换比较模型后费用和变化随之更新，换 session 后比较模型不变。
- 有子代理用其他模型的 session，Other models 行出现，两边总价都包含它。
- Cost tab 打开时状态栏显示所选模型下的合计和变化百分比，改过滤后跟着变。
- History 行的 Cost tab 显示空状态，不报错。

## Tech Design

### 现状

- 价格：`crates/core/src/pricing.rs` 的 `price(model)` 是一个 match，`cost(&UsageRow)` 给一次 API 响应计价；Haiku 5.5 的长 prompt 档用 `ModelPrice.long_prompt: Option<(u64, f64)>`。没有 Fable 5.2。
- 用量：`api_usage` 表一行一次响应（`session_id, agent, msg_id, model, input, cache_write_5m, cache_write_1h, cache_read, output, web_search, fast, cost_usd`），`Db::usage(session_id)` 可取单个 session 的全部行。`sessions` 表上的 `tokens_in` 只是未缓存输入。
- `SessionSummary`（core `model.rs`）经 `Db::list_sessions` / `session` / `find_by_prefix` 读出，`attach_details` 补工具计数和文件；app 的 `SessionRow::from_summary` 是唯一转换点。
- 表格列在 `crates/app/src/table.rs`（列宽常量、`FIXED_COLS_W`、`compare`），过滤在 app `model.rs::Filter`，工具栏下拉在 `toolbar.rs`，视图状态在 `settings.rs::ViewState`，详情 tab 在 `detail.rs::DetailTab`，状态栏在 `main.rs::render_status_bar`。

### 方案

core 在索引时按「模型 × fast × 是否长 prompt」把 `api_usage` 分组汇总存进 `session_model_usage`，读 session 时挂在 `SessionSummary.model_usage` 上；计价拆成按 token 种类的明细（`CostLines`），新增 `cost.rs` 用分组用量算出「主模型实际费用 / 换成目标模型的费用 / 其他模型原价」。app 直接从 `SessionRow` 上的分组用量计算 Cost tab 和状态栏合计，不走新的后台请求。

关键决策：

- **分组用量在索引时存表，随 session 一起读出**：初版在读 session 时现做 GROUP BY（9.7 万行 → 2,821 组），实测热缓存 65ms、冷缓存 350ms，全部压在启动首屏上（app 日志里 index read 498ms–1.46s）。改为由 `refresh_session_usage`（用量变化的唯一汇总入口）同时重写这张小表，启动只按主键顺序读几千行，index read 回到 37ms、首屏 340–409ms。备选「Cost tab 打开时向 store 要明细」要新增请求 / 事件，还会被扫描阻塞，而状态栏合计需要所有行的数据，最终还是要全量读。
- **按「是否长 prompt」预先分桶**：长 prompt 档由 prompt 大小决定，与目标模型无关，所以分组时按固定阈值 `LONG_PROMPT = 100_000` 切开，换成 Haiku 5.5 时也能精确计价。`ModelPrice.long_prompt` 改为只存倍率，阈值成为常量（目前只有 Haiku 5.5 有此档）。
- **主模型 = 费用最高的模型**（未计价的按 0，平局比调用次数），Model 过滤和重新计价用同一个定义，口径一致。备选「主线程最后一个模型（`context_model`）」不含子代理费用，与账单关系弱。
- **fast 调用换模型后**：用目标模型的 `fast_mult`，没有 fast mode 的模型为 1（按标准价）。
- **状态栏合计只在 Cost tab 打开时出现**：比较模型是 Cost tab 的状态，合计和它放在一起，平时状态栏不变。

### 数据变动

- 新表 `session_model_usage(session_id, model, fast, long_prompt, responses, input, cache_write_5m, cache_write_1h, cache_read, output, web_search, cost_usd, PRIMARY KEY(session_id, model, fast, long_prompt)) WITHOUT ROWID`，只由 `refresh_session_usage` 写；`clear_all`、`delete_session` 一并清理。加上 `pricing.rs` 的 Fable 5.2 价格，`SCHEMA_VERSION` 3 → 4，旧索引整库重建。
- `SessionSummary` 新字段 `model_usage: Vec<ModelUsage>`（`#[serde(default)]`，按 model、fast、long_prompt 排序）。
- `ViewState` 新字段 `model: Option<String>`（Model 过滤）、`compare_model: Option<String>`（比较模型），旧文件缺字段取默认。

### 逻辑变动

**core**

- `model.rs`：新增 `Tokens { input, cache_write_5m, cache_write_1h, cache_read, output, web_search }`（`prompt()`、`add`）和 `ModelUsage { model, fast, long_prompt, responses, tokens, cost_usd }`；`UsageRow::tokens()`；`SessionSummary::input_tokens_total()`（未缓存 + 缓存写 + 缓存读）；`set_usage` 同时填 `model_usage`，与索引读出的一致。
- `pricing.rs`：Fable 5.2 / Mythos 5.2；`LONG_PROMPT`；`CostLines { input, cache_write_5m, cache_write_1h, cache_read, output, web_search }`（`input_total()`、`total()`）；`price_tokens(&ModelPrice, &Tokens, fast, long_prompt) -> CostLines`，`cost()` 改为调用它；`display_name(model)`（`claude-opus-5-5` → `Opus 5.5`）；`resolve(name)`（补 `claude-` 前缀）；`CURRENT_MODELS` / `EARLIER_MODELS` 两个比较候选列表。
- `cost.rs`（新）：`primary_model(&[ModelUsage])`；`compare(&[ModelUsage], target) -> Option<Comparison>`，`Comparison { source, target, responses, fast_responses, long_prompt_responses, tokens, actual: Option<CostLines>, predicted: CostLines, others: Vec<OtherModel>, others_usd }`，`actual_total()` / `predicted_total()`。目标不在价格表或没有用量时为 `None`。
- `index/db.rs`：`refresh_session_usage` 先删后 `INSERT … SELECT … GROUP BY` 重写该 session 的 `session_model_usage`；`attach_details` 读整表填 `model_usage`，`session()` 按 `session_id` 读。
- `bin/asm.rs`：`cost` 加 `--model`（按主模型过滤）和 `--as`（每行加预测费用与变化，合计处按 token 种类列出实际 / 预测）。

**app**

- `model.rs`：`SessionRow` 加 `input_tokens`、`input_fmt`、`output_tokens`、`output_fmt`、`primary_model`、`model_usage`；`Filter.model`；`model_entries(rows)`（主模型 → 名称、session 数，按数量降序）。
- `table.rs`：`In`（52px）、`Out`（48px）列放在 Ctx 后；`PREVIEW_MIN_W` 200 → 160；`compare` 加 `input` / `output`；`recompute` 在设有比较模型时同时算出 `shown_projection`（实际合计、预测合计，只算两者都有的行）；`set_compare_model(Option<target>)`。
- `toolbar.rs`：Model 下拉与 `set_model_filter`。
- `cost_tab.rs`（新）：`render_cost` 与比较模型下拉；`detail.rs` 加 `DetailTab::Cost`、第三个 tab。
- `main.rs`：`compare_model` 状态与 `set_compare_model`；tab 切换时把比较模型同步给表格；状态栏追加合计；视图状态存取 `model` / `compare_model`；`ASM_COST_AUTOPLAY` 截图流程。
- `settings.rs`：两个新字段。
- `scripts/screenshot-cost.sh` + `make screenshot-cost`。

### 实施步骤

- [x] 1. core 测试：Fable 5.2 价格、`display_name`、`resolve`、`price_tokens` 拆分与 `cost` 一致；`compare` 的主模型选择、其他模型原价、fast、长 prompt；索引读出的 `model_usage`
- [x] 2. core 实现，`cargo test -p agent-session-core` 通过
- [x] 3. `asm cost --model --as` + CLI 测试；在真实索引上跑出 Opus 5.5 → Fable 5.2 的对比
- [x] 4. app：`SessionRow` / `Filter` / `model_entries` 测试与实现；In / Out 列；Model 下拉；设置项
- [x] 5. app：Cost tab、比较模型、状态栏合计
- [x] 6. autoplay 截图验收
- [x] 7. 文档（AGENTS.md、product-prd 状态表、本 plan）、commit
- [x] 8. 提交后发现：开发中途先把版本升到 4、后加 `session_model_usage`，中间的构建跑过真实索引（截图脚本用真实索引），留下「版本 4、缺这张表」的索引，提交的代码打开它即报 `no such table: session_model_usage`。`db.rs::ensure_schema` 改为同时比对 `sqlite_master` 与本版本在内存库建出的 schema，不一致就重建（测试 `an_index_whose_schema_differs_from_this_build_is_rebuilt_even_if_the_version_matches`）；本机真实索引的副本打开后重建约 14 秒，正确的 v4 索引原样保留，比对每次打开约多 1ms

### 测试与验证

- `tests/pricing.rs`：Fable 5.2 输入 / 输出 / 缓存读、5m / 1h 缓存写；`price_tokens` 各行之和等于 `cost`；`display_name`；`resolve`。
- `tests/cost.rs`（新）：两个模型时主模型取费用高的；只换主模型，其他模型原价计入两边总价；fast 调用换到有 / 无 fast mode 的模型；长 prompt 桶换成 Haiku 5.5 按 5 倍；Opus 5.5 → Fable 5.2 输入输出翻倍、缓存读不变；未知目标模型返回 `None`。
- `tests/index.rs`：索引后 `list_sessions` 与 `session()` 的 `model_usage` 一致，按 fast / 长 prompt 分组正确，含子代理用量。
- `tests/cli.rs`：`asm cost --as` 输出包含目标模型列和合计对比；`--model` 只留主模型匹配的 session。
- app `model.rs` 测试：Model 过滤、`model_entries` 排序、In / Out 格式。
- 截图：`make screenshot-cost` 过滤 Opus 5.5 → 按 Ctx 降序 → 选第一行 → Cost tab（Fable 5.2）→ 切 Sonnet 5.5 → 一个有子代理其他模型的 session。

### 风险与待定

- 列多了以后 Preview 变窄：默认 1440 宽、详情打开时不搜索没有横向滚动（截图 1–4）；搜索时多出 Matches 列，最右的 Kind 列会被挤出一部分，要横向滚动或拉宽窗口。
- 主模型按费用定，个别中途换模型的 session 会被归到花钱多的那个模型。
- 预测只换单价、不换 token 数；Fable 5.2 与 Opus 5.5 分词器相同（据 Anthropic 文档推断 token 数基本不变），但轮数和输出长度会变，只能当价格参考。

### 实测（2026-11-10，本机）

`asm cost --model opus-5-5 --as …`，主模型为 Opus 5.5 的 1,363 个 session，合计 $7,328.67：

| 换成 | 合计 | 变化 | 说明 |
|---|---|---|---|
| Fable 5.2 | $11,200.86 | +53% | 输入、缓存写、输出都 ×2，缓存读同价（$3,406，占原费用 47%） |
| Sonnet 5.5 | $3,678 | −50% | 所有单价减半 |
| Haiku 5.5 | $1,276 | −83% | 绝大多数调用 prompt 超过 100K，按 5 倍档计 |

单个 session 换成 Fable 5.2 的涨幅：中位数 +58%，P10 +12%，P90 +99%，范围 +5% 到 +100%。缓存读占比越高涨得越少（83% context 的 m1-web +33%），缓存命中差、以缓存写和输出为主的短 session 接近翻倍。截图在 `tmp/2026-11-10-cost-analysis/`（`make screenshot-cost`）。
