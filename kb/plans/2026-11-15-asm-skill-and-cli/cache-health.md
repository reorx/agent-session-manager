---
created: 2026-11-15
tags:
  - cli
  - skill
  - cache
  - cost
  - agent-session-manager
  - plan
---

# P2 缓存检查：`asm cache` 的总览与单个会话明细，修正两处失效误判，skill 加上缓存检查

总 plan 见 [index.md](index.md)；依赖 [session-search.md](session-search.md) 落地的通用约定（颜色、JSON、时间参数、排除当前会话）和 `skills/asm/references/cli.md`。

## PRD

### 背景

用户会请 agent「看看最近的会话缓存命中得好不好」。agent 要回答：一段时间里输入 token 有多少是读的缓存、多少是写缓存、多少没走缓存；缓存重建了几次、各是什么原因、花了多少钱；哪些会话最值得看；哪些重建本可以避免、怎么避免。

现在 app 表格有 Hit / Hit% 列，详情的回合卡会标出缓存重建，但 CLI 只有 `asm cost`（费用和 context），看不到命中率，更看不到重建；而且有两种情况会让重建判断出错（`kb/known-issues.md`）：

- 少数响应的用量数字只写在明细里、外层全是 0：这次调用漏算了费用，还被误报成一次重建，紧接着那次真正的重建又被漏掉。本机 60 天 3 次。
- `/rewind` 回到更早的回合后，下一次调用被报成一次原因不明的重建；官方文档说明这时读的是更早的缓存，不是失效。本机 60 天 38 次「不明」里有 5 次是它。

本机 30 天的基线（`kb/plans/2026-11-12-columns-and-cache-breaks.md`）：主线调用 72,719 次，重建 455 次；其中空闲超过缓存时长 282 次（重写 4920 万 token），compact 之后 145 次，换模型 5 次，原因不明 23 次。

### 目标

agent 用一两条命令拿到缓存情况的总览和最值得看的会话，再用一条命令看清某个会话在哪里、为什么重建，结合 skill 里的判断标准和官方文档给出建议。

### 功能与交互

**名词**（CLI、skill、报告共用）

- 命中率：读缓存的 token ÷ 全部输入 token（未缓存 + 写缓存 + 读缓存），和 app 表格的 Hit% 同一口径。
- 重建：主线上的一次调用，比上一次调用的输入少读了至少 1 万 token 的缓存，且不是 `/rewind` 造成的。原因四选一：空闲超过缓存时长（给出间隔和时长）、compact 之后、换了模型（给出原模型）、不明。
- 重建花费：这次调用写缓存的费用，和回合卡上显示的金额一致。

**1. `asm cache`：总览**

```
asm cache [--days 7 | --since DATE [--until DATE]] [--project P]... [--model M] [--include-self]
          [--by day|project|model] [--sort rebuild-cost|hit|tokens] [-n 10] [--json]
```

- 默认最近 7 天。统计范围内发生的调用（会话可以开始得更早）；subagent 的调用计入 token 总量，不判断重建。
- 输出（示意）：

```
last 7 days (2026-11-08 – 2026-11-14) · 412 sessions · 18,233 main-thread calls

input 1.42B tokens: cache read 96.6% · cache write 2.9% (1h 97%) · uncached 0.5%
rebuilds 96 · 9.8M tokens written · $61.20
  idle past TTL   58 · $40.10   (median gap 1h 42m)
  compaction      31 · $15.00
  model switch     2 · $0.90
  unknown          5 · $5.20

top sessions by rebuild cost:
  1b2bda5d  11-13 08:06  cw-edge         hit 91.2%  rebuilds 7  $9.80  现场抓帧任务停止排查
  …
```

- `--by day|project|model`：在总览下面多一张分组表，每行是命中率、写缓存占比、重建次数和花费。
- `--sort`：会话列表按重建花费（默认）、命中率（从低到高）、输入 token 排序；`-n` 控制列出几个。
- 范围内没有调用：输出 `0 calls in range` 和所用的范围。

**2. `asm cache ID`：单个会话**

- 头部：标题、项目、时间、命中率、输入 token 分布、重建次数和花费。
- 每次重建一行：回合号、时间、原因（含间隔与缓存时长或原模型）、少读的 token、写入的 token、花费。回合号与 `asm transcript --turns` 一致。
- `--calls`：列出每次主线调用：时间、回合号、模型、输入大小、读 / 写（5m / 1h）/ 未缓存、是否重建。
- 全部支持 `--json`。前缀不唯一时同 `asm session`。

**3. 修正两处误判（app 也受益）**

- 只在明细里有数字的响应：费用按明细计入，重建按明细判断。改完后这几个会话的 Cost、In、Hit 等数字会略有变化（索引重建一次）。
- `/rewind` 之后的调用不再报成重建；app 回合卡上对应的那条琥珀色说明消失。

**4. skill 加上缓存检查**

- `SKILL.md` 的场景路由加上「检查缓存命中」，指向新的参考文档。
- `references/cache-health.md`：名词和口径；怎么读 `asm cache` 的输出；本机基线和判断标准（什么算正常、什么值得提）；每种原因的解释和对策，依据官方文档（缓存时长的设置方法和优先级、订阅与 API key 的默认时长、换模型的几种隐蔽来源、MCP 服务器连上新工具、升级 Claude Code 后的第一轮、空闲时自动 compact）；报告格式；一个完整的例子。
- `references/cli.md` 加上 `asm cache` 的规范。

### 不做的事

- 不改 app 界面，不加「重建次数」列（以后可以直接用这一期存下的数据）。
- 不判断 subagent 的重建。
- 不自动修改用户的 Claude Code 设置，只给建议。
- 不估算「如果全部命中能省多少」，只报重建的写缓存花费。
- 不解释原因不明的重建的细节（记录里看不出 CLAUDE.md 变化、工具列表变化等）。

### 验收标准

- `asm cache` 默认给出最近 7 天的总览、原因分布和最该看的会话；`--by day` / `--by project` / `--by model` 给出分组表；`--json` 可解析。
- 任取 3 个会话，`asm cache ID` 的命中率与 app 表格的 Hit% 一致，重建列表与 app 回合卡上的说明一致（次数、原因、金额）。
- 本机 60 天里那 3 个只在明细里有数字的响应，费用计入了，不再被误报，被它掩盖的那次重建出现了。
- 本机 60 天里那 5 次 `/rewind` 后的误报不再出现；其余重建的次数和原因与改动前一致（差异逐条解释）。
- 总览查询不读原始记录文件，30 天范围在 1 秒内返回。
- 在本仓库新开的 Claude Code 会话里问「看看最近 7 天缓存命中得好不好」，agent 加载 skill，给出命中率、重建次数和原因、花费、最值得看的会话和有依据的建议，数字与 `asm cache` 一致。

## Tech Design

### 现状

- 重建判断只在 `claude/conversation.rs::Reader::cache_check`：按 `message.id` 去重的主线调用（跳过 `<synthetic>`），`CacheTrack { responses, last: (prompt, model, ts), hour, compacted }`；`dropped = 上次 prompt − 本次 cache_read`，`≥ CACHE_BREAK_MIN (10_000)` 记一次 `CacheBreak { ts, dropped, written, cost_usd, cause }`，挂在当前回合；原因优先级 compact → 换模型 → 间隔超过缓存时长（最近一次有写入的调用是 1h 就按 60 分钟，否则 5 分钟）→ 不明。compact 由 `system/compact_boundary` 或 `isCompactSummary` 的 user 记录触发。
- app 的 `crates/app/src/conversation.rs::cache_note` 把 `CacheBreak` 渲染成回合卡说明，摘要里统计 `rebuilds`。
- 用量：`claude/parse.rs::usage_row` / `tokens_of` 只读 `usage` 的外层字段；`RawUsage` 没有 `iterations`。`UsageSink` 保留每个响应第一条记录的位置和时间。`on_usage` 用 `row.prompt_tokens()` 更新 `context_tokens` / `context_peak`（只看非 sidechain）。
- `api_usage`（主键 `session_id, agent, msg_id`，WITHOUT ROWID）没有记录位置；`Db::insert_usage` 冲突时 `ts = min(..)`、其余覆盖。`refresh_session_usage` 从 `api_usage` 汇总写 `sessions` 的 token / 费用列和 `session_model_usage`。
- 索引解析时 `classify` 把 `system` 记录当作 Skip（`META_TYPES` 不含 `system`），所以索引看不到 `compact_boundary`；`isCompactSummary` 的 user 记录会经过 `on_user`。
- 已有的对照脚本：`tmp/cache-breaks.py`、`tmp/cache-breaks-causes.py`（用 `parentUuid` 祖先关系识别 rewind）、`tmp/cache-breaks-per-session.py`、`tmp/cache_audit.py`（iterations 问题）。

### 方案

把重建判断从 `Reader` 里抽成 core 的一个小状态机 `CacheTracker`，app 的回合卡和索引共用。索引时多存两样东西：每次调用在文件里的位置与判断用的 token 数（`api_usage` 加列），以及 compact 的位置（新表）。每个会话写完用量后，从这两张表按位置合并、喂给 `CacheTracker`，把结果整体重写进 `cache_breaks` 表，做法与 `refresh_session_usage` 相同：不在增量解析里维持状态，「全量 == 增量」自然成立。`asm cache` 只查表。

关键决定：

- **重建存表、按会话从表里重算**（用户 2026-11-15 决定存表）。被否决：在 `SessionDelta` 里增量追踪（要跨扫描保存上次调用的状态，和「全量 == 增量」的约束冲突）；查询时读原文件（30 天要读 5 GB）。
- **rewind 先试「写入量」判据**：rewind 后的调用读的是更早的缓存前缀，只写入新增的那一小段；真正的失效会把整个前缀重新写一遍。规则改为：`dropped ≥ 10k` 且（compact 之后，或本次写入 `≥ 10k`）才算重建。compact 之后照旧算（compact 后系统提示层可能仍在缓存里，写入量可能不到 1 万）。第 2 步用 `tmp/cache-breaks-causes.py` 的 5 次 rewind 和 60 天全部重建验证：rewind 全部消失、其余不变才采用。不成立时退回祖先关系方案：解析时维护 `uuid → 最近的调用` 映射，为每次调用记下沿 `parentUuid` 的上一次调用（`prev_msg_id`），跨增量的断点处退回按位置比较。
- **iterations**：`usage.iterations` 非空时，计费 token 按各项求和（compaction 项也计费，见 `kb/refs/claude-api-compaction-threshold.md`）；判断用的「本次 prompt」取最后一个 `message` 项的输入合计，「本次读缓存」也取该项。context 列同样取最后一个 `message` 项。实现前先在本机那 3 个响应上核对各项的数字，与文档的语义一致再落地。
- **compact 位置只用 `isCompactSummary`**，不改 `classify`（AGENTS.md：改字节预过滤要全量验证）。第 1 步在全部真实记录上确认每个 `compact_boundary` 之后、下一次调用之前都有一条 `isCompactSummary` 记录；不成立再考虑放行 `system` 记录。

### 数据变动

- `api_usage` 加列：`byte_offset INTEGER NOT NULL`（响应第一条记录的位置）、`prompt_tokens INTEGER NOT NULL`（本次调用的输入合计，iterations 时取最后一个 message 项）、`prompt_cache_read INTEGER NOT NULL`（判断用的读缓存）。`insert_usage` 冲突时 `byte_offset = min(..)`，与 `ts` 一样保留第一条记录的值。
- 新表 `session_compactions (session_id TEXT, byte_offset INTEGER, ts INTEGER, PRIMARY KEY (session_id, byte_offset)) WITHOUT ROWID`，只记主记录文件。
- 新表 `cache_breaks (session_id, msg_id, byte_offset, ts, model, dropped, written, cost_usd REAL, cause TEXT CHECK (cause IN ('idle','compaction','model_switch','unknown')), from_model TEXT, gap_ms INTEGER, ttl_ms INTEGER, PRIMARY KEY (session_id, msg_id)) WITHOUT ROWID`，加 `cache_breaks_ts` 索引。只由 `refresh_cache_breaks` 写。
- `SCHEMA_VERSION` +1（P1 之后是 6），整库重建。改 iterations 解析本身也要求提升版本（费用存在 `api_usage` 里）。
- `model.rs`：`UsageRow` 加 `byte_offset`、`prompt_tokens`、`prompt_cache_read`；`UsageRow::prompt_tokens()` 方法改为读字段（或重命名，调用处一并改）。`CacheBreak` 加 `msg_id`、`model`（索引需要；app 不用）。

### 逻辑变动

**core**

- 新模块 `claude/cache.rs`（或 `cache.rs`）：`CallUsage { msg_id, byte_offset, ts, model, fast, prompt_tokens, prompt_cache_read, cache_write_5m, cache_write_1h }`；`CacheTracker::compacted(&mut self)`、`CacheTracker::call(&mut self, &CallUsage) -> Option<CacheBreak>`，包含去重、1h / 5m 判断、原因优先级、rewind 规则、写缓存费用（`pricing::price_tokens`，含 fast 和长 prompt，口径同现在）。`CACHE_BREAK_MIN` 移到这里。
- `conversation.rs`：`CacheTrack` 和 `cache_check` 删掉，改为从 `message.usage` 构造 `CallUsage` 交给 `CacheTracker`，返回的重建挂到当前回合；compact 时调用 `compacted()`。回合卡的输出除两处修正外不变。
- `parse.rs`：`RawUsage` 加 `iterations`（各项的 `type` 和 token 字段）；`tokens_of` 按上面的规则求和；`usage_row` 产出新字段（`on_usage` 传入行位置）；`on_user` 遇到 `isCompactSummary` 时往 `Feed` 里加一条 compact 位置（`Feed.compactions: Vec<(u64, i64)>`）。
- `index/db.rs`：`insert_usage` 写新列；`insert_compactions` / `delete_compactions`；`refresh_cache_breaks(session_id)`：`SELECT … FROM api_usage WHERE session_id = ? AND agent = '' ORDER BY byte_offset` 与 `session_compactions` 按位置归并，喂 `CacheTracker`，删旧行写新行；查询：`cache_overview(&CacheFilter)`（按调用时间范围汇总 token：`api_usage` 含 subagent；重建：`cache_breaks`）、`cache_groups(&CacheFilter, by)`、`cache_sessions(&CacheFilter, sort, limit)`、`session_cache(session_id)`（重建列表；`--calls` 时读 `api_usage` 主线行）。
- `index/indexer.rs`：`write_parsed` 写完用量和 compact 后调用 `refresh_cache_breaks`；会话被删除、整份重解析（Rebuilt）时一并清掉该会话的 `session_compactions` / `cache_breaks`（找到 `delete_usage` 的所有调用处同步处理）。subagent 文件不产生 compact 和重建。
- 回合号：`cache_breaks` 不存回合号，CLI 输出时按 P1 的办法从 `messages` 里数 `kind = 'prompt'` 的行。

**CLI**

- `bin/asm/cache.rs`：`asm cache [ID]`，复用 P1 的 `RangeArgs`（默认 7 天）、`SessionFilterArgs`（`--project`、`--model`、`--include-self`）、渲染与 JSON 约定。时间范围按调用时间（`api_usage.ts`）过滤；会话列表里的数字也只算范围内的调用；`asm cache ID` 看整个会话。
- `references/cli.md` 先补 `### asm cache`，规范一致性测试覆盖它。

**skill**

- `SKILL.md`：场景路由加缓存检查；`references/cache-health.md` 按 PRD 第 4 点写，对策部分逐条引用 `kb/refs/claude-code-prompt-caching.md` 对应小节的线上地址（skill 对外发布，不能引用仓库内路径）。

### 实施步骤

- [ ] 1. 在全部真实记录上核对：`compact_boundary` 与 `isCompactSummary` 的对应关系；那 3 个 iterations 响应的各项数字；结果记进 session 总结
- [ ] 2. 用 `tmp/cache-breaks-causes.py` 的数据验证 rewind 的写入量判据（5 次 rewind 是否都 `written < 10k`，其余重建是否都不受影响）；决定采用它还是祖先关系方案
- [ ] 3. `CacheTracker` 抽出：先把现有 `cache_check` 的行为写成 `CacheTracker` 的单元测试（四种原因、1h / 5m、去重、synthetic），迁移后 `tests/conversation.rs` 照常通过；再加 rewind 与 iterations 的测试，实现修正
- [ ] 4. iterations 解析：fixture（`gen_fixtures.py` 加一个外层全 0、明细有数的响应，和一个带 compaction 项的响应）→ 测试 → 实现；提升 `SCHEMA_VERSION`
- [ ] 5. 存表：`api_usage` 新列、`session_compactions`、`cache_breaks`、`refresh_cache_breaks`；测试「全量 == 增量」（一次解析与分两次追加的结果相同，含响应被切成两半的情况）、删除与重解析时的清理、索引里的重建与 `read_conversation` 的结果一致
- [ ] 6. `cli.md` 加 `asm cache` → `asm cache` 总览、分组、会话列表、`asm cache ID`、`--calls`、`--json` + CLI 测试
- [ ] 7. `cache-health.md`、`SKILL.md` 路由
- [ ] 8. 真实数据验收（见下），新开 Claude Code 会话验收 skill
- [ ] 9. 删掉 `kb/known-issues.md` 里修好的两条；更新 `AGENTS.md`、`kb/docs/product-prd.md`、`kb/docs/claude-session-data-format.md`（iterations 口径），commit

### 测试与验证

| 验收标准 | 怎么验 |
|---|---|
| 总览、分组、JSON | CLI 测试（fixture 里放 compact、空闲超时、换模型各一次）；真实数据人工核对一次 |
| 与 app 的 Hit% 和回合卡一致 | 测试：fixture 上索引里的 `cache_breaks` 与 `read_conversation` 的 `cache_breaks` 逐条相同；真实数据取 3 个会话对照 app（截图或 `asm session-old --conversation` 的输出） |
| iterations 的 3 个响应 | 修正前后各跑一次 `tmp/cache_audit.py` 与 `asm cache ID`，记录差异 |
| rewind 的 5 次误报 | 修正前后对 60 天数据各统计一次重建（按会话、按原因），差异逐条解释，存进 `tmp/<date>-asm-p2/` |
| 30 天总览 1 秒内 | 真实索引上量 3 次 |
| skill 端到端 | 在本仓库新开 Claude Code 会话（herdr 新 tab）提问，记录命令序列和回答 |

### 风险与待定

- 写入量判据可能误伤：rewind 之后用户贴了很长的新内容（≥ 1 万 token）会被算成重建。验证数据里没有这种情况就接受，在 `cache-health.md` 里说明。
- iterations 的语义只有 3 个本机样本，若各项数字和文档对不上，先只修费用（按各项求和），判断规则保持取外层，记进 known-issues。
- `api_usage` 加列后变大（本机行数约等于调用次数），对启动读表没有影响（启动读的是 `session_model_usage`），但重建索引会慢一点；量一次 `asm index --rebuild` 的耗时并记录。
