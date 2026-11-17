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

- 默认最近 7 天。统计范围内发生的调用（会话可以开始得更早）；subagent 的调用计入 token 总量，不判断重建。`--model` 按范围内调用的规范化模型过滤，包含 subagent token；重建按当次调用模型归组，不按会话主模型或 model switch 的原模型。
- 时间沿用 P1：`--days N` 为滚动 N×24h（支持小数和 0），`--since` / `--until` 为本地午夜，区间 [since, until)，支持单独 until；`--by day` 按本地自然日，与 app Usage 的 4:00 用量日不同。
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
- 范围内没有调用：输出 `0 calls in range` 和所用的范围。零分母命中率为 JSON null / 文本 `—`；未知模型金额为 null / `unpriced`，不能当作 0。limit 只截会话榜单，不截总数和分组。

**2. `asm cache ID`：单个会话**

- 头部：标题、项目、时间、命中率、输入 token 分布、重建次数和花费。
- 每次重建一行：回合号、时间、原因（含间隔与缓存时长或原模型）、少读的 token、写入的 token、花费。回合号与 `asm transcript --turns` 一致。
- `--calls`：列出每次主线调用：时间、回合号、模型、输入大小、读 / 写（5m / 1h）/ 未缓存、是否重建。
- 全部支持 `--json`。前缀不唯一时同 `asm session`。ID 模式看整个会话，拒绝只适用于总览的过滤、分组、排序及榜单限制参数，不静默忽略。history-only 和无调用输出空明细；缺失 transcript 仍显示已存事实。首个 prompt 前及 AUTO 卡片中的调用编号为 null / `—`，不造第 0 回合。

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
- 总览查询本身不读原始记录文件，热索引 `--no-scan` 的 30 天范围三次均在 1 秒内返回；默认刷新另计增量扫描时间。
- 在本仓库新开的 Claude Code 会话里问「看看最近 7 天缓存命中得好不好」，agent 加载 skill，给出命中率、重建次数和原因、花费、最值得看的会话和有依据的建议，数字与 `asm cache` 一致。

## Tech Design

### 现状

- 已核对 `f961f5e`，P2-0 证据在 `tmp/2026-11-14-asm-skill-cli/asmcli-p2-explore/report.md`（T1–T16）。core `claude/conversation.rs::Reader` 从记录拼出 `CallInput`，按顺序调用 `CacheTracker::compacted`、`new_turn` 和 `call`；app 只消费、渲染 `CacheBreak`。
- 完整输入是 msg_id、parent、nullable 回复 / 请求开始时间、model、nullable effort / speed、全部计费 Tokens、判定 prompt / read 和 nullable RequestFacts（shape、betas、sign_in、keep、prev_len）。无请求与字段全缺的请求不同；None、0、false、空 beta 集合不能混淆。缺任一开始时间时整对退回回复时间；无时间间隔为 0。
- `UsageSink` 保留的是 Vec 顺序和首次时间，不是 byte_offset。计费同响应末条覆盖，`insert_usage` 的 ts 取 min；Reader 缓存则保留首个经过 RecordFilter 的合格响应。两套快照不可共用覆盖策略。
- `tokens_of` 是全部 iterations 之和，`prompt_of` 是最后一个 message 项。决定 37：本期仅存判定 prompt，不改 `UsageRow::prompt_tokens`、context、长 prompt、pricing、分组或账本口径；后者留待独立调整。
- 当前 compact 全量核对：主记录 509 boundary / 509 summary，subagent 5 / 5，两向缺失为 0；只用 summary 在当前语料成立，但不能保证未来 boundary-only 记录。

### 方案

索引保存按 byte_offset 排列的精简缓存事件（独立 `cache_events`），作为完整断点恢复；从表内事件重建 RecordFilter、祖先图和请求状态，再喂既有 CacheTracker，整体重写 `cache_calls` 与 `cache_breaks`。重放不读原文件，不改变判定算法、计费覆盖策略及单写线程事务顺序。

- **完整祖先与去重事实**：保存所有可被 Reader 解码记录的 uuid、parentUuid、两组去重类别和 sidechain 标记，含 tool-only user、无正文 assistant、同响应多个块的 uuid。先按 RecordFilter 去重（即使 sidechain 被剔除也占据 uuid），再更新图。沿已存图找祖先，只有原始链确实缺失或达到 Reader 的步数上限才走 Tracker 既有回退；不允许扫描断点造成差异。
- **按位置保存请求事件**：主键 `(session_id, byte_offset)`，request_id 可空，重复 ID 按位置覆盖。先更新每个 querySource（含独立 None 桶）的 last_len，再处理 ID；长度为 keep.unwrap_or(0) 与 tail 数量的 u32 饱和相加。保存 shape 原字符串及规范 beta 集合，只在重放内计算 Reader 使用的指纹，不持久化 DefaultHasher 的跨版本哈希；tail 正文与 blob 内容均不保存。
- **事件包含回合和 compact**：同时保留 boundary、summary、new_turn 与 AUTO 卡片边界，复现 compact→call→新 prompt 的清标记行为。编号随事件保存，缺失文件也可恢复；AUTO / 首 prompt 前的调用为 None。混合 tool_result 的 user 先按 Reader 处理工具结果，不误开回合。
- **独立缓存投影**：缓存事件通过共享的 `transcript::Record` 字段级降级解析读取，位于搜索/计费预过滤之前，保证节点不被既有 tool-only / metadata 过滤丢掉。原 classify、搜索与计费路径不改；缓存投影只留下语义和图节点，不索引正文。真实全量核对缓存投影及 Reader 一致性，额外量读取成本。
- 使用统一事件表替代独立 api_requests / session_compactions / ancestry 表，使无 ID 请求、重复 ID、重复 uuid 的历史顺序可精确恢复。被否决：仅给 api_usage 加列（首次缓存事实会被末次计费覆盖）、只存最终祖先图（未来节点会改变历史解析）、断点缺图就退回上一调用（违反一致性）。

### 数据变动

- `cache_events (session_id, byte_offset, data)`：WITHOUT ROWID，session 外键级联删除。data 为强类型精简事件 JSON：uuid / parent / user_input / sidechain 及 Node、Request、Call、Compact、Turn、Auto 的有效载荷。Call 保留首条候选的 nullable ts、model、tokens、prompt/read、effort/speed、requestRef；每个合格候选块均保存，以恢复 uuid→msg_id。
- `cache_calls`：以 `(session_id, msg_id)` 为键保存重放确定的首条缓存快照，含 byte_offset、nullable ts / turn、model、parent_msg_id、判定 prompt / read 及精简 payload。与 api_usage 的末条计费快照分离。
- `cache_breaks`：以 `(session_id, msg_id)` 为键，byte_offset、nullable ts / turn、model、dropped、written、nullable cost_usd、cause 和完整 CacheBreak payload；显式原因码覆盖 compaction / model_switch / effort / fast_mode / sign_in / request_settings / idle / system_or_tools / earlier_messages / request_unchanged / unknown，附带 from/to/on/gap/ttl 无损往返，时间索引 `cache_breaks_ts`。
- `model.rs` 添加缓存事件、调用与 `IndexedCacheBreak` 包装类型；CacheBreak 原公共字段不变，只增加序列化支持。api_usage、sessions 列及 SESSION_COLUMNS / row_to_session 的列序不变。
- `SCHEMA_VERSION` 7 → 8，索引整库重建；账本不删不迁移，salvage 仍用原计费结构。

### 逻辑变动

**core**

- `claude/cache.rs` 判定不改；`conversation.rs` Reader 保持独立，作为逐条对照基准。
- 新增缓存事件投影 / 重放模块，复用 Record、RecordFilter、opens_turn、tokens_of、prompt_of；feed 返回 cache_events，subagent usage_only 不采集缓存事件。
- `index` 独立缓存存取模块提供 `insert_cache_events`、`delete_cache_facts`、`refresh_cache_breaks`、`cache_events`、`cache_calls`、`cache_breaks`。每次追加仅写新事件，再按会话重放全部事件；只在写入事务内刷新派生表。
- `indexer::write_parsed` 主文件非 Append 时清旧缓存事实；upsert session 后写事件并重放。文件仅缺失时保留事实、调用和重建（与 usage 保留同步）；真删 session / clear_all / schema 重建级联清理；subagent 单独删除只改计费用量，不改主线缓存。

**CLI（P2-2）**

- 先写 `references/cli.md` 的 asm cache 契约（默认值、空集合、错误、排序并列键），再实现命令与聚合。复用已有 RangeArgs、当前会话排除、项目解析、前缀查找、JSON 错误约定和 --no-scan / 锁回退；当前不存在 SessionFilterArgs，有实际复用需求再提取。
- 调用级时间 / model 过滤，token 从 api_usage（含 subagent）汇总，重建从 cache_breaks 汇总，两边先按键聚合再合并，避免一对多 JOIN 放大费用。根据 EXPLAIN QUERY PLAN 决定 api_usage 时间索引。
- `--calls` 读 cache_calls 的首条缓存输入，费用与总 Hit% 仍按 api_usage 计费事实；不加模型 / 时间裁剪的全会话命中率才能直接与 app 表格比较。

**skill（P2-3）**

- SKILL.md 路由、references/cache-health.md 与 PRD 一致；官方依据引用线上地址，示例全部虚构。

### 实施步骤

- [x] 1. 核实 cache-break-accuracy 完成后的现状。
  - 2026-11-17，证据：`tmp/2026-11-14-asm-skill-cli/asmcli-p2-explore/report.md` §2（T1–T16）。
- [x] 2. 全量 compact 对应及 api-request 分类核对。
  - 2026-11-17，证据：同报告 §3、`audit-frozen.log` / `audit.json`（1,346,307 行，零差异）。
- [x] 3. 缓存事实持久化、重放、全量 == 各断点增量、Reader 对照、生命周期清理、冻结 842 条审计、大小与耗时测量。
  - 2026-11-17，证据：[P2-1 总结](../../sessions/2026-11-17-asm-p2-store.md)；681 项测试通过，842 条 TSV 零差异，parity 第二轮 21/21。
- [ ] 4. cli.md → cache 聚合与命令、JSON、CLI 行为测试、规范反向检查。
- [ ] 5. cache-health skill、路由及相关文档（含决定 33 的虚构例子清理）。
- [ ] 6. 真实数据与新会话 skill 验收。
- [ ] 7. 协调者更新共享文档和状态、安排 CLI 安装及收尾。

### 测试与验证

- 先红后绿：全部 11 原因，延迟 compact、compact→调用→新 prompt，rewind 跨断点、tool-only 节点、同 msg_id 多块（usage 相同 / 不同），同 uuid 两组及 sidechain，请求缺字段 / 错类型 / 无 ID / 重复 ID / 缺 querySource，缺时间 / 时间倒序、EOF 半行、替换 / 截断 / 缺失 / 真删 / subagent 单独删除、未知模型金额。
- 按每条记录与半行分割，比较落库事件、首条调用和重建的所有字段；与 read_conversation 逐条比较重建和编号，不只比较数量。
- 冻结审计复用 `tmp/2026-11-16-cache-break-accuracy/cache-break-ref/{manifest.json,calls.jsonl,after.tsv}`，缺失文件用 `cbreak-tracker/r1/manifest.json` 已验证相同 SHA256 的备用前缀。3,398 文件、133,186 calls、842 rebuilds；完整键、原因、时间、量和金额对照。不用今日滚动范围替代，旧 30 天 455 次仅为旧算法历史基线。
- `make test`、账本回归、隔离索引 `asm cost --compare` 与基线一致；parity 两次，遵循 brief 的动态会话差异判定。
- 同一冻结副本测 HEAD 与完整存表版本：空库冷建（不宣称清 OS 缓存）、checkpoint 后索引大小、增量与重放时间。30 天聚合在 P2-2 用热库 --no-scan 量三次，另列默认扫描时间。
- 所有 scratch 在 `tmp/2026-11-14-asm-skill-cli/asmcli-p2-store/`，真实索引和账本不写入；不运行 GUI、不安装全局 CLI。

### 风险与待定

- 保存完整顺序和图会增加索引与扫描成本，按同输入实测；不能用探索阶段仅放行请求、尚未落库的零体积增量外推。
- 新缓存事实与计费首次 / 末次策略分离，异常多块响应可能呈现不同 token，属于各自既有语义，不可为统一显示改变 CacheTracker 或账本。
- context / 长 prompt 切换到最后 message iteration 留待独立工作，本期严格不改（决定 37）。
