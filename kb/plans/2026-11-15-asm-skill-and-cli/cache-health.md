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

# P2 缓存检查：`asm cache` 的总览与单个会话明细，重建存进索引，skill 加上缓存检查

总 plan 见 [index.md](index.md)；依赖 [session-search.md](session-search.md) 落地的通用约定（颜色、JSON、时间参数、排除当前会话）和 `skills/asm/references/cli.md`。重建的判定规则和共用的 `CacheTracker` 来自 [cache-break-accuracy](../2026-11-16-cache-break-accuracy.md)，它先做，本期不改判定（用户 2026-11-16 决定）。

## PRD

### 背景

用户会请 agent「看看最近的会话缓存命中得好不好」。agent 要回答：一段时间里输入 token 有多少是读的缓存、多少是写缓存、多少没走缓存；缓存重建了几次、各是什么原因、花了多少钱；哪些会话最值得看；哪些重建本可以避免、怎么避免。

现在 app 表格有 Hit / Hit% 列，详情的回合卡会标出缓存重建，但 CLI 只有 `asm cost`（费用和 context），看不到命中率，更看不到重建。重建判断里的两处误报（只写在明细里的用量、`/rewind`）和按请求记录细分的原因，由 [cache-break-accuracy](../2026-11-16-cache-break-accuracy.md) 先改好；本期只把它的结果存进索引、做成 CLI 和 skill。

本机 30 天的基线（`kb/plans/2026-11-12-columns-and-cache-breaks.md`）：主线调用 72,719 次，重建 455 次；其中空闲超过缓存时长 282 次（重写 4920 万 token），compact 之后 145 次，换模型 5 次，原因不明 23 次。

### 目标

agent 用一两条命令拿到缓存情况的总览和最值得看的会话，再用一条命令看清某个会话在哪里、为什么重建，结合 skill 里的判断标准和官方文档给出建议。

### 功能与交互

**名词**（CLI、skill、报告共用）

- 命中率：读缓存的 token ÷ 全部输入 token（未缓存 + 写缓存 + 读缓存），和 app 表格的 Hit% 同一口径。
- 重建：主线上的一次调用，比它接续的那次调用（rewind 之后是 rewind 回到的那一点）少读了至少 1 万 token 的缓存。原因和判断顺序见 [cache-break-accuracy](../2026-11-16-cache-break-accuracy.md) 的「失效原因」表：compact 之后、换模型（给出原模型）、换 effort、切换 fast mode、登录方式变了、请求设置变了、空闲超过缓存时长（给出间隔和时长）、system 或工具变了、更早的消息被改写、请求没变却没命中、不明。与 app 回合卡是同一套。
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
  (every cause with at least one rebuild gets a line)

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

**3. 判定口径**

- 索引和 app 回合卡用同一个判定（[cache-break-accuracy](../2026-11-16-cache-break-accuracy.md) 抽出的 `CacheTracker`），本期不改判定；同一个会话，`asm cache ID` 的重建与回合卡逐条相同。

**4. skill 加上缓存检查**

- `SKILL.md` 的场景路由加上「检查缓存命中」，指向新的参考文档。
- `references/cache-health.md`：名词和口径；怎么读 `asm cache` 的输出；本机基线和判断标准（什么算正常、什么值得提）；每种原因的解释和对策，依据官方文档（缓存时长的设置方法和优先级、订阅与 API key 的默认时长、换模型的几种隐蔽来源、MCP 服务器连上新工具、升级 Claude Code 后的第一轮、空闲时自动 compact）；报告格式；一个完整的例子。
- `references/cli.md` 加上 `asm cache` 的规范。

### 不做的事

- 不改 app 界面，不加「重建次数」列（以后可以直接用这一期存下的数据）。
- 不判断 subagent 的重建。
- 不自动修改用户的 Claude Code 设置，只给建议。
- 不估算「如果全部命中能省多少」，只报重建的写缓存花费。
- 不改判定规则和原因（归 [cache-break-accuracy](../2026-11-16-cache-break-accuracy.md)）。

### 验收标准

- `asm cache` 默认给出最近 7 天的总览、原因分布和最该看的会话；`--by day` / `--by project` / `--by model` 给出分组表；`--json` 可解析。
- 任取 3 个会话，`asm cache ID` 的命中率与 app 表格的 Hit% 一致，重建列表与 app 回合卡上的说明一致（次数、原因、金额）。
- 索引里 60 天的重建（次数、原因、金额）与 cache-break-accuracy 真实数据核对时输出的 TSV 逐条一致。
- 总览查询不读原始记录文件，30 天范围在 1 秒内返回。
- 在本仓库新开的 Claude Code 会话里问「看看最近 7 天缓存命中得好不好」，agent 加载 skill，给出命中率、重建次数和原因、花费、最值得看的会话和有依据的建议，数字与 `asm cache` 一致。

## Tech Design

### 现状

- 以下按 [cache-break-accuracy](../2026-11-16-cache-break-accuracy.md) 完成之后的代码写；开工前对照代码核实。重建判断在 `claude/cache.rs::CacheTracker`：`compacted()` 和 `call(&CallInput) -> Option<CacheBreak>`。`CallInput` 是 msg_id、接续调用的 msg_id（沿 `parentUuid` 找到）、回复时间与请求开始时间、模型、effort、speed、计费 token、判定用的 prompt 与读缓存，以及请求事实（system 与工具指纹、beta 哈希、登录标记、`keep`、同一来源上一次请求的长度，来自 `api-request` 记录）。app 的 `conversation.rs::Reader` 从记录里拼出它。
- app 的 `crates/app/src/conversation.rs::cache_note` 把 `CacheBreak` 渲染成回合卡说明，摘要里统计 `rebuilds`。
- 用量：`claude/parse.rs::tokens_of` 按 `iterations` 各项求和（计费口径），`prompt_of` 取最后一个 `message` 项（判定口径）。`UsageSink` 保留每个响应第一条记录的位置和时间。`on_usage` 用 `row.prompt_tokens()` 更新 `context_tokens` / `context_peak`（只看非 sidechain）。
- `api_usage`（主键 `session_id, agent, msg_id`，WITHOUT ROWID）没有记录位置；`Db::insert_usage` 冲突时 `ts = min(..)`、其余覆盖。`refresh_session_usage` 从 `api_usage` 汇总写 `sessions` 的 token / 费用列和 `session_model_usage`。
- 索引解析时 `classify` 把 `system` 记录当作 Skip（`META_TYPES` 不含 `system`），所以索引看不到 `compact_boundary`；`isCompactSummary` 的 user 记录会经过 `on_user`。
- 已有的对照脚本：`tmp/cache-breaks.py`、`tmp/cache-breaks-per-session.py`、`tmp/2026-11-16-cache-accuracy-plan/`（请求记录的覆盖与语义），以及 cache-break-accuracy 的参考脚本和它输出的 TSV。

### 方案

`CacheTracker` 由 cache-break-accuracy 抽出，app 的回合卡和索引共用。索引时多存三样东西：每次调用在文件里的位置和 `CallInput` 的各项（`api_usage` 加列）、请求记录的事实（新表），以及 compact 的位置（新表）。每个会话写完用量后，从这两张表按位置合并、喂给 `CacheTracker`，把结果整体重写进 `cache_breaks` 表，做法与 `refresh_session_usage` 相同：不在增量解析里维持状态，「全量 == 增量」自然成立。`asm cache` 只查表。

关键决定：

- **重建存表、按会话从表里重算**（用户 2026-11-15 决定存表）。被否决：在 `SessionDelta` 里增量追踪（要跨扫描保存上次调用的状态，和「全量 == 增量」的约束冲突）；查询时读原文件（30 天要读 5 GB）。
- **接续调用在解析时算好存列**：解析时维护 `uuid → parentUuid` 和 `uuid → 调用` 的映射，为每次调用记下沿 `parentUuid` 找到的接续调用（`parent_msg_id`）。增量解析从断点继续时前面的记录不在内存里，这时记为空，`CacheTracker` 退回按位置比较上一次调用。
- **请求事实**：`classify` 放行 `api-request` 行（不放行很大的 `api-request-blob` 行，二者按 `type` 的完整字符串区分），按 AGENTS.md 的要求在全部真实记录上验证。同一来源上一次请求的长度在 `refresh_cache_breaks` 里按位置从请求表算，不在增量解析里维持状态。
- **compact 位置只用 `isCompactSummary`**，不改 `classify`（AGENTS.md：改字节预过滤要全量验证）。第 2 步在全部真实记录上确认每个 `compact_boundary` 之后、下一次调用之前都有一条 `isCompactSummary` 记录；不成立再考虑放行 `system` 记录。

### 数据变动

- `api_usage` 加列：`byte_offset INTEGER NOT NULL`（响应第一条记录的位置）、`prompt_tokens INTEGER NOT NULL`（`prompt_of` 的 prompt：最后一个 message 项的输入合计）、`prompt_cache_read INTEGER NOT NULL`（判定用的读缓存）、`parent_msg_id TEXT`、`request_id TEXT`（`requestRef`）、`effort TEXT`、`speed TEXT`。`insert_usage` 冲突时 `byte_offset = min(..)`，与 `ts` 一样保留第一条记录的值。
- 新表 `api_requests (session_id, request_id, byte_offset, ts, query_source, shape_hash, betas_hash, sign_in, keep, len, PRIMARY KEY (session_id, request_id)) WITHOUT ROWID`，只记主记录文件。
- 新表 `session_compactions (session_id TEXT, byte_offset INTEGER, ts INTEGER, PRIMARY KEY (session_id, byte_offset)) WITHOUT ROWID`，只记主记录文件。
- 新表 `cache_breaks (session_id, msg_id, byte_offset, ts, model, dropped, written, cost_usd REAL, cause TEXT CHECK (cause IN ('compaction','model_switch','effort','fast_mode','sign_in','request_settings','idle','system_or_tools','earlier_messages','request_unchanged','unknown')), from_value TEXT, to_value TEXT, gap_ms INTEGER, ttl_ms INTEGER, PRIMARY KEY (session_id, msg_id)) WITHOUT ROWID`，加 `cache_breaks_ts` 索引。只由 `refresh_cache_breaks` 写。
- `SCHEMA_VERSION` 在 cache-break-accuracy 之后再 +1，整库重建。
- `model.rs`：`UsageRow` 加 `byte_offset`、`prompt_tokens`、`prompt_cache_read`；`UsageRow::prompt_tokens()` 方法改为读字段（或重命名，调用处一并改）。`CacheBreak` 加 `msg_id`、`model`（索引需要；app 不用）。

### 逻辑变动

**core**

- `claude/cache.rs`、`conversation.rs` 在 cache-break-accuracy 里已经改好，本期不动。
- `parse.rs`：`usage_row` 产出新字段（`on_usage` 传入行位置；`parent_msg_id` 用上面的映射）；`classify` 放行 `api-request`，解析出请求事实交给 `Feed`；`on_user` 遇到 `isCompactSummary` 时往 `Feed` 里加一条 compact 位置（`Feed.compactions: Vec<(u64, i64)>`）。
- `index/db.rs`：`insert_usage` 写新列；`insert_compactions` / `delete_compactions`；`insert_requests` / `delete_requests`；`refresh_cache_breaks(session_id)`：`SELECT … FROM api_usage WHERE session_id = ? AND agent = '' ORDER BY byte_offset` 与 `session_compactions`、`api_requests` 按位置归并，拼出 `CallInput` 喂 `CacheTracker`，删旧行写新行；查询：`cache_overview(&CacheFilter)`（按调用时间范围汇总 token：`api_usage` 含 subagent；重建：`cache_breaks`）、`cache_groups(&CacheFilter, by)`、`cache_sessions(&CacheFilter, sort, limit)`、`session_cache(session_id)`（重建列表；`--calls` 时读 `api_usage` 主线行）。
- `index/indexer.rs`：`write_parsed` 写完用量和 compact 后调用 `refresh_cache_breaks`；会话被删除、整份重解析（Rebuilt）时一并清掉该会话的 `session_compactions` / `cache_breaks`（找到 `delete_usage` 的所有调用处同步处理）。subagent 文件不产生 compact 和重建。
- 回合号：`cache_breaks` 不存回合号，CLI 输出时按 P1 的办法从 `messages` 里数 `kind = 'prompt'` 的行。

**CLI**

- `bin/asm/cache.rs`：`asm cache [ID]`，复用 P1 的 `RangeArgs`（默认 7 天）、`SessionFilterArgs`（`--project`、`--model`、`--include-self`）、渲染与 JSON 约定。时间范围按调用时间（`api_usage.ts`）过滤；会话列表里的数字也只算范围内的调用；`asm cache ID` 看整个会话。
- `references/cli.md` 先补 `### asm cache`，规范一致性测试覆盖它。

**skill**

- `SKILL.md`：场景路由加缓存检查；`references/cache-health.md` 按 PRD 第 4 点写，对策部分逐条引用 `kb/refs/claude-code-prompt-caching.md` 对应小节的线上地址（skill 对外发布，不能引用仓库内路径）。

### 实施步骤

- [ ] 1. 前置：[cache-break-accuracy](../2026-11-16-cache-break-accuracy.md) 完成；对照它落地后的代码核实本节的「现状」
- [ ] 2. 在全部真实记录上核对：`compact_boundary` 与 `isCompactSummary` 的对应关系；放行 `api-request` 后 `classify` 的结果（不误放 `api-request-blob`）；结果记进 session 总结
- [ ] 3. 存表（含 `api_requests` 和 `parent_msg_id`）：`api_usage` 新列、`session_compactions`、`cache_breaks`、`refresh_cache_breaks`；测试「全量 == 增量」（一次解析与分两次追加的结果相同，含响应被切成两半的情况）、删除与重解析时的清理、索引里的重建与 `read_conversation` 的结果一致
- [ ] 4. `cli.md` 加 `asm cache` → `asm cache` 总览、分组、会话列表、`asm cache ID`、`--calls`、`--json` + CLI 测试
- [ ] 5. `cache-health.md`、`SKILL.md` 路由
- [ ] 6. 真实数据验收（见下），新开 Claude Code 会话验收 skill
- [ ] 7. 更新 `AGENTS.md`、`kb/docs/product-prd.md`、`kb/docs/claude-session-data-format.md`（索引里的请求记录），commit

### 测试与验证

| 验收标准 | 怎么验 |
|---|---|
| 总览、分组、JSON | CLI 测试（fixture 里放 compact、空闲超时、换模型各一次）；真实数据人工核对一次 |
| 与 app 的 Hit% 和回合卡一致 | 测试：fixture 上索引里的 `cache_breaks` 与 `read_conversation` 的 `cache_breaks` 逐条相同；真实数据取 3 个会话对照 app（截图或 `asm session-old --conversation` 的输出） |
| 与 cache-break-accuracy 的结果一致 | 索引里 60 天的 `cache_breaks` 与它真实数据核对时输出的 TSV 逐条相同，差异存进 `tmp/<date>-asm-p2/` |
| 增量与断点 | 「全量 == 增量」测试；真实数据上增量扫描后的 `cache_breaks` 与整库重建后的相同（断点处退回按位置比较，差异逐条看） |
| 30 天总览 1 秒内 | 真实索引上量 3 次 |
| skill 端到端 | 在本仓库新开 Claude Code 会话（herdr 新 tab）提问，记录命令序列和回答 |

### 风险与待定

- 增量断点处没有接续调用，rewind 恰好跨断点时索引会与 app 不一致（app 读整份文件）。正在写的会话才有断点，影响面小；验证数据里出现了就记进 known-issues。
- `api_usage` 加列后变大（本机行数约等于调用次数），对启动读表没有影响（启动读的是 `session_model_usage`），但重建索引会慢一点；量一次 `asm index --rebuild` 的耗时并记录。
