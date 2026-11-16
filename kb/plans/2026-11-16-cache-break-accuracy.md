---
created: 2026-11-16
tags:
  - cache
  - cost
  - conversation
  - parser
  - agent-session-manager
  - plan
---

# 缓存失效判定与用量核算：修正两处误判，用请求记录说清失效原因

## PRD

### 背景

2026-11-12 对照官方文档（`kb/refs/`）核对了费用和缓存的算法：价格表、缓存读写倍率、Hit% 的口径、按 5 分钟 / 1 小时判断缓存时长，都和官方一致。问题出在少数响应的用量读取，和回合卡上「缓存为什么重建」的判断。

本机最近 60 天（按记录文件的修改时间）有 127,132 次主线调用，按现在的规则判出 811 次缓存重建：空闲超过缓存时长 494 次、compact 之后 249 次、换模型 33 次、原因不明 35 次。（2026-11-17 实施步骤 1 复核：这组数字出自已经修正了 iterations 的前期脚本，不是现行实现的输出。验收的基线以 `tmp/2026-11-16-cache-break-accuracy/cache-break-ref/report.md` 和它的 `before.tsv` 为准。）逐个查下来有五个问题：

1. **用量只写在明细里的响应**：本机有 4 次 Opus 5.5 响应（d074913f、01dacff6、603da7ec、196366dd），用量的外层数字全是 0，真实数字只写在明细里。结果有三处出错：
   - 读缓存（每次约 36 万 token）和输出没有计费；
   - 回合卡把这次调用误报成一次「原因不明」的重建；
   - 紧接着那次真正的重建（只读回约 3 万）反而被漏掉。
   按官方语义，以后明细里还可能出现服务端 compact 的那一项，它要计费，但不算在外层数字里。
2. **`/rewind` 之后的误报**：rewind 回到更早的回合后，下一次调用读的是更早的缓存，这是官方文档里明确说「不失效」的操作。现在却和文件里的上一次调用比，丢掉的那段被算成「少读」，于是报一次重建。35 次「不明」里有 4 次是它；另有 4 次空闲重建也发生在 rewind 之后，比较的对象也错了。
3. **「不明」其实大多能说清**：Claude Code 从 2.1.300 起，每次请求前都在记录文件里写一条请求记录，60 天里 79% 的主线调用带它。记录里有：
   - 请求的开始时间；
   - system 和工具定义的指纹；
   - 和上一次请求相比，开头有几条消息没变；
   - 请求用的 beta 设置。
   回复记录上另有 effort。用这些数据重查带请求记录的「不明」：
   - 11-02 的 7 次：请求一会儿带订阅登录的标记，一会儿不带，几秒内切走又切回。推测是登录方式在订阅和 API key 之间切换了，而缓存按组织隔离。
   - 11-06 的 6 次：都是 vibench 的测试会话。请求完全没变，也没过期，但第二次调用没读到第一次刚写入的缓存。
   - 其余几次：system 或工具定义变了（包括 `/cd`），或者 Claude Code 改写了更早的消息。
   2.1.300 之前的会话没有请求记录，剩约 6 次只能仍算不明。
4. **空闲间隔的起点不对**：官方说缓存寿命从请求**开始**时算，现在用的是两次回复记录的时间。用请求记录里的开始时间核对了 332 次空闲重建，判定全部不变，只是回合卡上显示的间隔有偏差。（2026-11-12 曾用「回复前一条记录的时间」代替开始时间，得出 15 次会改判，那是代替值出错：记录的先后顺序不总是时间顺序。）
5. **缓存时长的判断不对**：现在按「最近一次有写入的调用」写的是 1 小时还是 5 分钟来判断。本机主线有 14 次，是在 1 小时的长前缀后面跟了一次只写 5 分钟的调用；从这次起，之后都按 5 分钟算，而前面那段 1 小时的缓存其实还在。

查过、但不能当成原因的线索（以后不用再查）：
- 记录里的 `cwd`：它会跟着 Bash 的 `cd` 变，60 天变了 14,796 次，只有 0.3% 伴随重建。真正的 `/cd` 会改 system，按 3 归为「system 或工具变了」。
- `speed` 字段从缺失变成 `standard`：这只是字段出现了，不是切换 fast。本机没有真正切换过 fast。
- 上下文里的图片数：已被 3 里的「更早的消息被改写」取代。
- Claude Code 版本变化：60 天里 8 次，其中伴随重建的都已经归为空闲、compact 或换模型，没有一次落在「不明」里。

和其他 plan 的分工（用户 2026-11-16 决定）：失效的判定逻辑全部放在本 plan，先做。`kb/plans/2026-11-15-asm-skill-and-cli/cache-health.md`（P2）只负责把判定结果存进索引、做 `asm cache` 和 skill，不再改判定。

### 目标

回合卡上每次缓存重建的原因和金额都对；带请求记录的会话里不再出现「原因不明」；只在明细里有用量的响应，Cost、In、Hit%、Ctx 也算对。

### 功能与交互

**1. 用量只在明细里的响应**

- 表格的 Cost、In、Hit、Hit%、Ctx，详情的 Metadata 和 Cost tab，都按明细计算。本机这 4 个会话的数字会略有变化（每个约多 $0.1）。
- 升级后第一次启动时索引自动重建一次（约 4 秒），表格照常先显示、再刷新。
- 回合卡上去掉这次误报，补上被它掩盖的那次重建。

**2. `/rewind` 之后**

- 和这次调用真正接续的那次调用比较，也就是 rewind 回到的那一点。只读回 rewind 点之前的部分，不算重建，回合卡上不出现说明行。
- rewind 之后如果真的重建了（比如隔了很久才回来），照常标出，原因和少读的量都按 rewind 回到的那一点算。

**3. 失效原因**

一次重建只标一个原因：按下表从上往下，取第一个成立的。前五种都会让请求换用另一份缓存；空闲超时意味着缓存已经不在了，此时改了什么都无所谓；后面几种只有缓存还在有效期内才可能是原因。

| 原因 | 什么情况 | 回合卡说明（示例） | 需要请求记录 |
|---|---|---|---|
| compact 之后 | compact 之后第一次真正的重建（compact 后紧接着的那次调用读回了缓存时，标在下一次重建上） | `after compaction` | 否 |
| 换模型 | 现有 | `model changed from Opus 4.7` | 否 |
| 换 effort | 两次调用的 effort 不同 | `effort changed from high to xhigh` | 否 |
| 切换 fast mode | 一次是 fast、一次是标准速度 | `fast mode turned on` / `fast mode turned off` | 否 |
| 登录方式变了 | 两次请求，一次带订阅登录的标记、一次不带 | `sign-in changed (subscription / API key)` | 是 |
| 其他请求设置变了 | 请求的其他 beta 设置不同 | `request settings changed` | 是 |
| 空闲超过缓存时长 | 现有，间隔和时长按下面第 4、5 条 | `idle 2h 10m, cache lasts 1h` | 否 |
| system 或工具变了 | 连上 / 断开 MCP、启用插件、`/cd` 等 | `system prompt or tools changed` | 是 |
| 更早的消息被改写 | Claude Code 改了已经发过的消息，比如裁掉旧图片 | `earlier messages changed` | 是 |
| 请求没变却没命中 | 以上都不成立，请求开头没变、缓存也没过期 | `cache missed, request unchanged` | 是 |
| 不明 | 没有请求记录，以上也都对不上 | `cause unknown` | — |

- 说明行的格式不变：`cache rebuilt · 写入量 · 金额 · 原因`，琥珀色，一个回合有几次就列几行。Conversation 顶部摘要里的重建次数照旧。
- 2.1.300 之前的会话没有请求记录，只能判断表中「需要请求记录」为「否」的几种，其余仍是不明。

**4. 空闲间隔**

- 有请求记录时，`idle …` 显示的是两次请求开始时间之间的间隔，这是官方口径；没有请求记录时照旧用回复记录的时间。

**5. 缓存时长**

- `cache lasts …` 和空闲判断，按丢掉的那一段当初写入时用的时长：前面写的是 1 小时，后面只写了一小段 5 分钟，那么闲置 10 分钟后丢的只是那一小段，不到 1 万 token 就不算重建；如果丢的正是 5 分钟那段，就显示 `cache lasts 5m`。

### 不做的事

- 不判断 subagent 的缓存（它们在各自的文件里）。
- 不读请求记录里保存的消息正文，也不指出具体是哪一条消息被改了。
- 不解释服务端为什么没命中。
- 不按模型细分「换 effort 会不会失效」：只有真的重建了，才标出 effort 变了。
- 不改界面布局、不加列，只改回合卡的说明文字。
- 不把重建存进索引，不加 CLI 命令，这两件归 P2（`cache-health.md`）。
- 不处理「自算费用比 Claude Code 记的低 1.95%」（另一条 known issue，原因是记录文件之外的请求）。

### 验收标准

- 4 个只在明细里有用量的响应：费用计入，`asm cost --compare` 的合计变化等于这 4 次的金额；回合卡上它们的误报消失，被掩盖的那次重建出现，原因是 compact。
- 4 次 rewind 后的「不明」：3 次不再报，1 次按 rewind 回到的那一点改判为换模型（真的换了模型，不能为了符合「不再报」把它藏掉）。另 4 次 rewind 后的空闲重建，按 rewind 回到的那一点重新判断，参考脚本的结果是 3 次仍为空闲、1 次换模型，逐条列出。
- 60 天全部重建的前后对比表：带请求记录的会话里没有「不明」（没有请求记录的会话里还剩约 9 次）；每种原因的次数与独立写的参考脚本一致；每一处改判都能逐条解释。精确数字以步骤 1 冻结的输入为准，不和本计划里的概数硬比。
- 带请求记录的 332 次空闲重建（其中 5 次在现行规则下是换模型），改用请求开始时间算间隔后仍都超过缓存时长；最终原因按原因表的顺序定，参考脚本的结果是 314 空闲、5 换模型、5 登录方式、8 请求设置。显示的间隔改为请求开始时间之间。
- 在 1 小时前缀后面跟 5 分钟写入的会话里，判断按丢掉那一段的时长。
- 截图：一个会话的回合卡上显示一种新原因的说明行。
- 用量账本：升级后重建索引，这 4 次响应在 `usage-ledger.sqlite` 里变成新的 token，所在 session 的小时汇总跟着变。Usage 视图里这几天的 Cost 变化等于 `asm cost --compare` 的合计变化，状态栏 Today 与 Usage 一致。
- `make test`、`make parity` 通过。

## Tech Design

### 现状

- **判定**只在 `crates/core/src/claude/conversation.rs`：
  - `Reader::assistant` 对每条 assistant 记录调用 `cache_check`。
  - `CacheTrack { responses, last: (prompt, model, ts), hour, compacted }` 按 `message.id` 去重，跳过 `SYNTHETIC_MODEL`。
  - 比较规则：`dropped = 上一次的 tokens.input_total() − 本次 cache_read`，`≥ CACHE_BREAK_MIN (10_000)` 记一次重建。
  - 原因依次是 compact → 换模型 → `ts` 之差超过缓存时长（`hour` 由最近一次有写入的调用决定）→ 不明。
  - 金额用 `pricing::price_tokens` 只算缓存写入，含 fast 和长 prompt。
  - `system/compact_boundary` 和 `isCompactSummary` 两种记录都会置 `compacted`。
- **读记录**：`read_conversation` 把每一行解析成 `transcript::Record`，经 `RecordFilter::keep`（同 uuid 去重、去 sidechain）后交给 `Reader::record`。`record` 遇到没有 `message` 的记录直接返回，所以 `api-request`、`attachment` 记录现在会被跳过（它们没有 uuid，能通过 filter）。
  - `Record` 没有 `parentUuid`、`requestRef`、`effort`，也没有请求记录的字段。
  - `Message.usage` 是 `parse.rs::RawUsage`，没有 `iterations`。
- **用量**：`parse.rs::tokens_of` 只读外层，5m / 1h 细分缺失时全算 5 分钟。
  - `usage_row` 产出 `UsageRow`，`pricing::cost` 用 `UsageRow::prompt_tokens()` 判断长 prompt，`on_usage` 用它更新 `context_tokens` / `context_peak`。
  - 改解析结果要提升 `index/db.rs::SCHEMA_VERSION`（现在是 6）。
  - 索引的 `classify` 会跳过 `attachment` 和 `api-request*` 行，本 plan 不改它。
- **类型**：`model.rs::CacheBreak { ts, dropped, written, cost_usd, cause }`，`CacheBreakCause::{Compaction, ModelSwitch { from }, Idle { gap_ms, ttl_ms }, Unknown}`。
- **界面**：app `crates/app/src/conversation.rs::cache_note` 把 `CacheBreak` 渲染成说明行，测试是 `a_turn_that_lost_the_cache_says_how_much_was_written_again_and_why`。
- **测试**：core 的 `crates/core/tests/conversation.rs` 用内联记录（`prompt_at`、`call`、`breaks`），记录没有 uuid。

**记录格式**（2026-11-12 至 11-16 在本机 60 天数据上核实，脚本在 `tmp/2026-11-16-cache-accuracy-plan/`）：

- **assistant 记录**：
  - `requestRef` 指向请求记录的 `id`；
  - `effort` 是字符串，几乎每条都有；
  - `uuid` / `parentUuid` 构成对话链。
- **`api-request` 记录**（2.1.300 起，2.1.288 及以前没有；主线回复能对上请求记录的占 79%）：
  - `id`；`timestamp` 是请求开始时间；
  - `querySource`：只见过 `repl_main_thread` 和 `sdk`；
  - `shapeHash`：system 和工具定义的指纹；
  - `params.betas`：beta 列表；`params.model`、`params.output_config.effort`；
  - `keep`：开头与同一来源上一次请求相同的消息条数。连续两次主线请求里，96,184 / 96,741 次等于上一次的 `keep + tail` 长度；
  - `tail`：其余消息的哈希；
  - `cacheControl[].value.ttl`：与用量里 5m / 1h 的细分一致（99.5k 次）。
- **`api-request-blob`**：按哈希存消息正文，行很大，不用。
- **`usage.iterations`**：130,936 次响应都有，全部只有一项 `message`；外层非 0 时与各项之和逐字段相等，只有 4 次外层全 0。各项都带 `cache_creation` 细分。

### 方案

把判定从 `Reader` 抽成 core 的 `claude/cache.rs::CacheTracker`。输入是不依赖记录类型的 `CallInput`，P2 可以直接用索引里的行喂它。

`Reader` 负责从记录里拼出每次调用的 `CallInput`：
- 用量按新的明细规则读；
- 沿 `parentUuid` 找到这次调用接续的那次调用；
- 经 `requestRef` 取得请求记录里的事实；
- 取得请求开始时间。

`CacheTracker` 拿当前调用和它接续的那次调用比较，按 PRD 表中的顺序定原因。它还记录缓存前缀里每一段的写入时长，用来判断丢掉的那一段活多久。

关键决定：

- **rewind 按祖先调用比较**：解析时记下每条记录的 `parentUuid`，从这次调用的第一条记录往上走，遇到的第一条属于其他调用的记录就是要比较的那次调用。链走不到任何调用时（compact 会断链；测试里的记录没有 uuid），退回比较文件里的上一次调用，也就是现在的行为。
  - 被否决：P2 原先想先试的「写入量 ≥ 1 万才算重建」。rewind 之后贴一大段新内容会被误判；少读的量仍以错误的调用为基准；祖先关系在所有版本的记录里都有，而且是确切的。
- **iterations**：
  - 计费：`iterations` 非空时，按各项求和。本机 130,932 次与外层相同，修正 4 次，也覆盖文档里 compaction 那一项要计费的情况。
  - 判定：用最后一个 `message` 项，它的 prompt 是三项输入之和，它的读缓存作为本次读缓存。
  - 索引里的 context 和长 prompt 判断仍用 `UsageRow::prompt_tokens()`（各项之和）。本机没有 compaction 项，两者完全相同；P2 加 `prompt_tokens` 列时改取最后一个 `message` 项。
  - 被否决：只在外层全 0 时才读明细。这样会漏掉 compaction 项。
- **请求事实只取记录里的计数、指纹、设置和时间**，不读 blob 的正文：够区分原因，而且不用解析大行。
- **`keep` 和同一 `querySource` 的上一次请求比较**，不和上一次调用的请求比较：两次调用之间可能有没留下回复的请求（中断、出错），`keep` 是相对于真正的上一次请求而言的。
- **请求开始时间**：取请求记录的 `timestamp`；没有请求记录时用回复记录的时间（现在的口径）。
  - 被否决：用「回复前一条记录的时间」代替。记录的先后顺序不总是时间顺序，2026-11-12 用它得出了 15 次错误的改判。
- **缓存时长按段算**：每次调用记下它之后缓存前缀的分段 `[(end, ttl)]`。取接续调用的分段，在本次读缓存的位置截断，再接上本次写入的 1 小时段和 5 分钟段（API 要求较长的 TTL 断点在前），相邻同时长的段合并，所以段数只有几段。
  - 判断空闲时，用接续调用分段里包含「本次读缓存位置」的那一段的时长：它在接续调用开始时被读到或写入过，寿命从那时起算。
  - 用量里的 5m / 1h 细分与请求声明的 TTL 一致，所以新旧会话都用细分，不读 `cacheControl`。
  - 被否决：「最近一次写入是哪种」的单一标志，也就是问题 5。
- **登录方式**：两次请求的 beta 列表里，`oauth-` 开头那一项一个有一个没有，就算「登录方式变了」，其余 beta 不同算「请求设置变了」。依据是 11-02 那 7 次只差 `oauth-2025-04-20` 和 `extended-cache-ttl-2025-04-11`。「切到 API key」是推测，所以文案只写 `sign-in changed`。
- **effort / fast 不需要请求记录**：`effort` 在回复记录上，`speed` 在用量里。两边都有值且不同才算变了，缺失不算，避免 `None → standard` 那种假变化。

### 数据变动

- `model.rs::CacheBreakCause` 新增变体（文档注释写清触发条件）：
  - `EffortChange { from: String, to: String }`
  - `FastMode { on: bool }`
  - `RequestSettings { sign_in: bool }`
  - `SystemOrTools`
  - `EarlierMessages`
  - `RequestUnchanged`

  `Idle { gap_ms, ttl_ms }` 的含义改为请求开始时间之间的间隔和丢失段的时长。`CacheBreak` 结构不变。
- `transcript.rs::Record` 新增字段：
  - `parent_uuid`（`parentUuid`）、`request_ref`（`requestRef`）、`effort`；
  - 请求记录的字段：`id`、`query_source`、`shape_hash`、`keep: Option<u32>`、`tail: Option<Vec<IgnoredAny>>`（只要长度，`IgnoredAny` 是零大小类型，不分配）、`params: Option<RequestParams { betas: Option<Vec<String>> }>`。
- `parse.rs::RawUsage` 新增 `iterations: Option<Vec<RawIteration>>`，`RawIteration { kind (type), input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens, cache_creation }`。
- `index/db.rs::SCHEMA_VERSION` +1，因为 4 次响应存进 `api_usage` 的费用变了。`schema.sql` 不变。
- 用量账本（`ledger.rs`、`ledger.sql`，用量统计第 1 期加的）不改结构、不加迁移。版本号变了以后，打开索引时 `build_schema` 先 `salvage_into_ledger` 把旧索引的用量抄进账本，接着全量解析，经 `Db::transaction` 把新 token 写进账本：`responses` 按 `(session_id, agent, msg_id)` upsert，覆盖 token；`write_rows` 对改过的 session 调 `refresh_hourly`，重算 `usage_hourly`。记录文件已经删掉的响应没有新的解析结果，保留旧值。这 4 次都在 60 天内，记录都还在。
- 不改索引的 `classify`、不存请求记录，这些归 P2。

### 逻辑变动

**core `claude/cache.rs`（新）**，在 `claude/mod.rs` 里注册：

- `CACHE_BREAK_MIN` 从 `conversation.rs` 移过来。
- 新类型：
  - `pub struct CallInput { msg_id, parent: Option<String>（接续调用的 msg_id）, ts: Option<i64>（回复时间，说明行挂的位置）, start_ts: Option<i64>, model, effort: Option<String>, speed: Option<String>, tokens: Tokens（计费口径）, prompt: u64, read: u64（判定口径）, request: Option<RequestFacts> }`
  - `pub struct RequestFacts { shape: u64, betas: u64, sign_in: bool, keep: u32, prev_len: Option<u32> }`：指纹和 beta 列表都存哈希，不存原文，调用多时内存小。
- `pub struct CacheTracker`，内部是 `HashMap<String, CallState>` 加上一次调用的 msg_id，以及 `compacted`。`CallState` 存 prompt、model、effort、speed、start_ts、ts、request 和分段。
- `pub fn compacted(&mut self)`。
- `pub fn call(&mut self, c: &CallInput) -> Option<CacheBreak>`，步骤：
  1. 去重：见过的 msg_id 返回 `None`。
  2. 基准 = `parent` 指向的调用，没有就用上一次调用；没有基准就只记状态。
  3. `dropped = 基准.prompt − c.read`，小于 `CACHE_BREAK_MIN` 不算重建。
  4. 按 PRD 的顺序定原因：
     - compact 用 `compacted`；
     - 换模型、effort、fast 比较各自的字段；
     - 登录方式 / 请求设置要求两边都有 `request`，比较 `sign_in`、`betas`；
     - 空闲：`gap = c.start_ts − 基准.start_ts`，任一缺失就用 `ts` 之差；`ttl` 取分段里包含 `c.read` 位置的那一段的时长。`c.read` 超出所有分段时取最后一段，没有分段时按 5 分钟；
     - system 或工具：比较 `shape`；
     - 更早的消息被改写：`c.request.keep < c.request.prev_len`；
     - 请求没变：两边都有 `request`，其余都不成立；
     - 否则不明。
  5. 金额照旧：本次的缓存写入 × `price_tokens`，fast 取 `c.speed == "fast"`，长 prompt 取 `c.prompt > LONG_PROMPT`。
  6. 记下本次的分段。本次算作一次重建（不论什么原因）时清掉 `compacted`；没有重建就保留，留给 compact 之后第一次真正的重建。新的用户回合开始时，如果 compact 之后已经有过调用，也清掉，避免很久以后的重建被归到 compact；compact 之后还没有任何调用就进入新回合的（手动 `/compact` 后接着发下一条消息，这是最常见的情况），标记要保留到这一回合的调用。（2026-11-17 更正：第一版写成「新回合一律清掉」，参考脚本据此把 125 次 compact 之后的重建改判成空闲等原因，违背了用户「标成 compact」的决定。）用户 2026-11-17 定：4 个会话里 compact 之后的第一次响应读回了约 36 万缓存，紧接着的那次才少读约 34 万，这次应标 compact，不按请求记录标成「更早的消息被改写」或「system 或工具」。

**core `claude/conversation.rs`**

- 删掉 `CacheTrack` 和 `cache_check`。`Reader` 改用 `cache: CacheTracker`，另加：
  - `parents: HashMap<String, Option<String>>`：uuid → parentUuid，每条有 uuid 的记录都记；
  - `call_of: HashMap<String, String>`：assistant 记录的 uuid → msg_id；
  - `requests: HashMap<String, (i64, RequestFacts)>`：请求 id → 开始时间和事实；
  - `last_len: HashMap<String, u32>`：querySource → 上一次请求的长度；
  - 当前回复的 msg_id。
- `record`：
  - 开头先把 uuid → parent 记下；
  - `kind == "api-request"` 时，算出 `prev_len = last_len[source]`，更新 `last_len[source] = keep + tail.len()`，存进 `requests`，返回；
  - 已有的 compact 分支改为调用 `self.cache.compacted()`。
- `assistant`：对每次回复的第一条记录组装 `CallInput`：
  - `parent`：从 `r.parent_uuid` 往上走 `parents`，遇到 `call_of` 里属于其他 msg_id 的记录就停；走到 `None` 或超过 10,000 步就是 `None`；
  - `start_ts` 和 `request` 由 `requestRef` 查 `requests` 得到；
  - `tokens` 用 `tokens_of`，`prompt` / `read` 用 `prompt_of`。

  记下本条的 `call_of`。`tracker.call` 返回的重建挂到当前回合，`ensure_turn` 照旧。`SYNTHETIC_MODEL` 和缺 id / model / usage 的记录照旧跳过。

**core `claude/parse.rs`**

- `tokens_of`：`iterations` 非空时，按各项的 input、写入细分（缺细分时全算 5 分钟）、读、output 求和；web search 仍取外层 `server_tool_use`。
- 新增 `pub(crate) fn prompt_of(u: &RawUsage) -> (u64, u64)`：取最后一个 `type == "message"` 的项，返回它的三项输入之和与读缓存；没有这一项就用外层。

**core 其他**

- `model.rs`：`CacheBreakCause` 的新变体。
- `index/db.rs`：`SCHEMA_VERSION` +1，在注释里写明原因。

**app `crates/app/src/conversation.rs`**

- `cache_note` 按 PRD 的表加上新原因的文案：
  - effort 原样显示（`high`、`xhigh`）；
  - 模型名照旧用 `display_name`；
  - `FastMode { on }` 显示 `fast mode turned on` / `off`；
  - `RequestSettings { sign_in: true }` 显示 `sign-in changed (subscription / API key)`，否则显示 `request settings changed`。

**文档**

- `kb/docs/claude-session-data-format.md`：
  - 记录类型表加上 `api-request` / `api-request-blob`；
  - assistant 记录的 `requestRef` / `effort`；
  - 用量一节写明 `iterations` 的计费和判定口径。
- `AGENTS.md`：「缓存失效」那一条改指 `claude/cache.rs`，写明按祖先调用比较、原因来自请求记录。core 规则里用量账本那一条（现在写的是「账本存 token 不存费用，改价格不用动账本」）补一句：改了解析出的 token，要靠提升 `SCHEMA_VERSION` 触发重建，把记录还在的响应覆盖成新值；记录已删的响应保留旧值。
- `kb/known-issues.md`：删掉「顶层 usage 全为 0」和「`/rewind` 之后」两条。
- `kb/docs/product-prd.md`：如果提到失效原因，就同步改。
- 修好后写 session 总结，内容包括：已解决的两条已知问题、前后对比表。

### 实施步骤

- [x] 1. **参考脚本**：在 `tmp/<date>-cache-break-accuracy/expected_breaks.py` 用 Python 独立实现本 plan 的规则，在 60 天数据上输出每次重建（会话、回复时间、原因、少读、写入、间隔、时长）的 TSV，同时按现行规则输出一份，生成前后对比表。可以复用 `tmp/2026-11-16-cache-accuracy-plan/` 里的脚本。
  - 2026-11-17 完成，已按更正后的 compact 规则更新（`tmp/2026-11-16-cache-break-accuracy/`：`expected_breaks.py` 与 19 个行为测试、冻结输入 `calls.jsonl` / `manifest.json`、`before.tsv` / `after.tsv`、`comparison.md`、`changes-explained.tsv`；报告 `cache-break-ref/report.md`，以「协调者摘要（cache-break-ref-r2）」为准）。60 天 133,186 次调用：现行 845 次重建，新规则 842 次（compact 276、空闲 490、换模型 35、不明 9、更早的消息 2、system 或工具 3、登录方式 11、请求设置 9、请求没变 7），差异 708 处，其中原因变化 46 处。
- [ ] 2. **iterations**：
  - `gen_fixtures.py` 加一个外层全 0、明细有数的响应，和一个带 `compaction` 项的响应，`uv run` 重跑并提交；
  - `tests/parse_transcript.rs` 写失败的测试（会话费用、token、context）；
  - 实现 `tokens_of` / `prompt_of`，提升 `SCHEMA_VERSION`；
  - 账本的测试：旧版本索引里外层全 0 的响应已经抄进账本，提升版本后打开，账本里的这次响应变成按 `iterations` 算的 token，`usage_hourly` 同步；记录文件已删的 session 保持旧值。
- [ ] 3. **`CacheTracker` 单元测试**（`cache.rs` 内）：
  - 先把现有行为写成测试：四种原因、1h / 5m、去重；
  - 再加：按祖先比较（rewind 不报、rewind 后空闲照报且基准正确）、分段时长（1h 前缀加 5m 小段）、开始时间口径、每种新原因、缺请求记录时退回不明；
  - 然后实现。
- [ ] 4. **接入 `Reader`**：
  - `Record` 新字段；`api-request` 的处理；`parents` / `call_of` 的维护与往上走；
  - `tests/conversation.rs` 新增带 uuid / `parentUuid` / `requestRef` / `api-request` 的内联记录测试；
  - 现有测试不改期望（没有 uuid 时退回文件顺序）。
- [ ] 5. **app**：`cache_note` 的测试加上新原因 → 实现。
- [ ] 6. **真实数据核对**：
  - 新增 `#[ignore]` 测试 `real_transcripts_cache_breaks`（`crates/core/tests/conversation.rs`）：`ASM_CACHE_AUDIT_DAYS=60 ASM_CACHE_AUDIT_OUT=<tsv>` 对 `~/.claude/projects/*/*.jsonl` 跑 `read_conversation`，输出与步骤 1 同格式的 TSV；
  - 与参考脚本逐条比对，差异逐条解释，存进任务的 tmp 目录。参考脚本先按 2026-11-17 定的 compact 规则更新（见步骤 1）。比对用步骤 1 冻结的 `calls.jsonl` / manifest 作输入，对齐毫秒单位、文件筛选和完整的调用键；要重扫实时数据，就用参考脚本重新生成一对新旧 TSV 再比；
  - 跑 `asm cost --compare`、`make test`、`make parity`；
  - 账本：在真实账本的副本上（`sqlite3 "file:…/usage-ledger.sqlite?mode=ro" ".backup <副本>"`，`ASM_DB` 指向副本旁的索引，不碰真实数据目录）重建一次，核对这 4 次响应的新值和 Usage 视图的变化；
  - 用 `asm session-old ID --conversation` 的 `read_ms` 在最大的几个会话上量读取耗时，与改动前比较。
- [ ] 7. **截图**：选一个第一处失效是新原因的会话，跑 `ASM_AUTOPLAY_CACHE_SESSION=<id> make screenshot-cost`，看回合卡说明行。
- [ ] 8. **收尾**：更新文档、known-issues，写 session 总结，commit。

### 测试与验证

| 验收标准 | 怎么验 |
|---|---|
| 只在明细里有用量的 4 次响应 | 步骤 2 的 fixture 测试；真实数据：步骤 6 的 TSV 里这 4 个会话的误报消失、下一次出现；`asm cost --compare` 合计的变化与 4 次的金额对上 |
| rewind | 步骤 3 和 4 的单元测试；真实数据：4 次「不明」消失，另 4 次按祖先重判的结果列表 |
| 60 天前后对比、带请求记录的会话里没有不明 | 步骤 1 与步骤 6 的 TSV 逐条一致；对比表存档 |
| 332 次空闲仍是空闲 | 对比表里带请求记录的空闲重建计数 |
| 分段时长 | 单元测试；真实数据里那 14 处之后的重建逐条看 |
| 说明行文案 | app 单元测试；步骤 7 的截图（用 `Read` 打开看过） |
| 不退化 | `make test`、`make parity`；读取耗时对比 |

### 风险与待定

- **记录格式**：`requestRef`、`api-request`、`effort` 都是 Claude Code 未公开的内部格式，可能改名或消失。缺字段时退回不需要请求记录的判断；解析失败的只是这些字段，不能影响回合卡的其他部分（字段一律 `Option`）。
- **`keep` 的语义**是从数据里推断的（96,184 / 96,741），不是文档写的。rewind 时 `keep` 相对的是被放弃那一支的最后一次请求，所以要先用祖先关系判断有没有重建，`keep` 只用来区分原因。实现时在步骤 1 的数据上确认，没有出现 rewind 被归为「更早的消息被改写」的情况。
- **「请求没变却没命中」**：
  - 它包括服务端的原因，也可能包括 Claude Code 改了记录之外的东西（比如请求头）。文案只陈述看得到的事实。
  - 「刚写入就没读到」的那 6 次推测是服务端的现象；推测依据是：同一小时、同一种测试会话。
- **祖先往上走的开销**：正常每次调用走几步。连着很多条没有调用的记录（大量 attachment）时会多走一些，有 10,000 步上限。`Reader` 多出两张按 uuid 建的表，大会话会多占几 MB，步骤 6 量过再定是否要优化。
- **iterations 里的 compaction 项**本机没有样本，只按文档实现、由 fixture 覆盖。真出现时要核对外层与各项的关系，看是否符合「外层不含 compaction 项」。
- **用量账本**（`kb/plans/2026-11-16-usage-stats/usage-ledger.md`，正在实现）：如果账本先上线，提升 `SCHEMA_VERSION` 时会先把旧索引的用量（包括那 4 次错的数字）抄进账本，重建后再由 `ON CONFLICT … DO UPDATE` 用新解析的数字覆盖。记录文件还在的会话会被改对；文件已删的会话本来也不在这 4 次里。步骤 6 要查一下账本里这 4 个 msg_id 的 token 是不是新的数字。
- **与 P2 的衔接**：P2 存表时要把 `CallInput` 的每个字段存下来，包括祖先调用的 msg_id、开始时间和请求事实，并让索引的 `classify` 放行 `api-request` 行（不放行 `api-request-blob`），按 AGENTS.md 的要求全量验证。这些写在 P2 的 spec 里，本 plan 不做。
