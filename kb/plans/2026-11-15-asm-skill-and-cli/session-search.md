---
created: 2026-11-15
tags:
  - cli
  - skill
  - search
  - agent-session-manager
  - plan
---

# P1 找会话：会话级搜索、会话卡片、按范围读会话，以及 skill 的第一版

总 plan 见 [index.md](index.md)；文本类别、会话级匹配、时间范围等共用概念在那里定义，这里不重复。

## PRD

### 背景

用户在 Claude Code 里请 agent 找过去的某个会话，agent 要做五件事：把描述拆成几个概念、给每个概念扩写说法（中英文、同义词、错误码）、决定搜哪类文本、按会话列出候选再挑几个核实、把结果报告给用户。现在的 `asm` 一件都支持得不好（见总 plan「背景」）。这一期把 CLI 补到 agent 能独立完成这五件事，并写好 skill 的第一版。

### 目标

agent 只靠 skill 和 `asm`，几条命令内找到用户要的会话，读到足以核实的那一段，给出可以直接恢复的命令。

### 功能与交互

**1. 旧命令改名**

`asm search` 改名为 `asm search-old`，`asm session` 改名为 `asm session-old`，参数和输出不变，继续和 Python 脚本逐字比对。原来的名字给下面的新命令。

**2. 新的 `asm search`：按会话找**

```
asm search TERM... [--in KINDS] [--tool NAME]... [--regex] [--same-message]
           [--days N | --since DATE [--until DATE]] [--project P]... [--model M] [--favorites]
           [--include-self] [--sort recent|hits] [-n 20] [--snippets 3] [--json]
```

- 每个 TERM 是一个概念，会话里要出现全部概念。TERM 内用 `|` 分隔几种说法，出现任一种即可。默认按字面匹配，不分大小写；`--regex` 时每个 TERM 是一个正则（`|` 仍是「任一」），写错的正则按字面匹配。
- `--same-message`：所有概念必须出现在同一条消息里。
- `--in`：搜哪些文本类别，逗号分隔，取值 `prompt`、`reply`、`thinking`、`tool`、`tool-input`、`tool-output`、`other`、`all`。默认 `prompt,reply,thinking`。`--tool Bash` 只看这个工具的调用（可重复），隐含 `tool`。
- 时间范围按会话的活动时间算：范围内有过活动的会话才参与，命中在会话里的任何位置都算。不给时间时：只搜索引里的类别（`prompt`、`reply`、`thinking`、`other`）不限时间；搜工具或用 `--regex` 时默认最近 30 天。实际范围写在输出第一行。
- `--project`：项目目录名包含这段文字（可重复，任一）；也可以直接给一个目录路径。
- `--model`：只看主模型是它的会话（主模型的定义同 `asm cost --model`）。
- 默认按最近活动时间排序；`--sort hits` 按命中总数排序。默认最多 20 个会话，每个会话最多 3 段片段。
- 当前会话默认不算在内；`--include-self` 时算。

文字输出（示意）：

```
2 sessions · terms [连不上|无法连接|offline] [健康检查|healthz] · in prompt,reply,thinking · all time · 0.03s

1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26
  updated 2026-11-14 17:09 · started 2026-11-13 07:40 · cw-edge (main) · 21 prompts · $4.10
  title: 现场抓帧任务停止排查
  hits: prompt 1 · reply 4 · thinking 0
  [reply · turn 6 · 11-13 08:06] …第 12 轮开始前健康检查«连不上»现场（`«healthz»_error:URLError`）…
  [prompt · turn 5 · 11-13 08:01] …为什么抓帧停了，«healthz» 报错…
  resume: cd /Users/reorx/Code/cw-edge && claude --resume 1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26

…
```

- 片段带文本类别、回合号和时间，命中处用 `«»` 标出；回合号和 `asm transcript --turns` 的编号一致。
- 没有命中：输出 `0 sessions`，下一行提示可以放宽什么（时间范围、文本类别、去掉一个概念）。
- 达到上限：最后一行写 `showing 20 of 57 sessions, use -n for more`。
- `--json`：同样的内容，外加完整的工作目录、记录文件路径、每段片段在原文件里的位置，字段见 CLI 规范。

**3. 新的 `asm session ID`：会话卡片**

给完整 ID 或唯一前缀，输出这个会话的元数据：标题、首条和末条 prompt、项目路径和工作目录、分支、开始 / 更新时间、回合数、prompt 数、常用工具、改过的文件、费用、token 与命中率、context 峰值、模型、会话种类（交互 / SDK / 后台）、是否收藏、记录文件路径、恢复命令。只有 history 记录的会话写明没有记录文件。前缀匹配到多个会话时，列出候选（ID、时间、标题）并以错误退出。支持 `--json`。

**4. 新的 `asm projects`：项目列表**

列出项目的路径、会话数、最近活动时间，按最近活动排序；`--days` / `--since` 只列范围内有活动的项目。用途：用户说「某个 app」时，agent 先看有哪些项目，再用 `--project` 限定。支持 `--json`。

**5. `asm transcript` 按范围读**

默认输出不变（整份导出）。新增：

- `--outline`：只列每个回合的编号、时间和用户输入的第一行，先看这个会话聊了什么。
- `--turns 5-7`：只输出这几个回合（也可以是单个回合 `5`）。
- `--grep TERM...`：只输出有命中的回合，TERM 的写法同 `asm search`；`--context 1` 带上前后各 1 个回合。保留的回合里只显示 prompt、命中的 item（一条回复、一段 thinking、带 `--tools` 时的一次工具调用等）和它前后各 1 个 item，其余折叠成一行占位，写明折叠了几个 item、多少字符；不带 `--grep` 的 `--turns` 照旧整回合。原因：agent 跑任务的会话常只有一个回合，只按回合过滤几乎不缩小输出（brief 决定 23）。
- `--thinking`：带上 thinking；`--tools`：工具调用给出输入和输出（每段输出最多 2,000 字符，超出的写明截掉了多少），而不只是次数。
- 用了这些选项时，每个回合标出编号和时间。

**6. 通用约定（所有命令）**

- 颜色：`--color auto|always|never`，默认 `auto`，输出不在终端或设了 `NO_COLOR` 时不带颜色；`--no-color` 保留，等于 `never`。
- 新命令的时间参数统一为 `--days N`、`--since YYYY-MM-DD`、`--until YYYY-MM-DD`（本地日期）。
- 拿不到索引的写锁时（app 正在写），跳过刷新，用现有索引查询，stderr 提示一行「index busy, results may miss the last few minutes」，不报错退出。

**7. skill 第一版**

仓库里的 `skills/asm/`：

- `SKILL.md`：skill 是做什么的、什么时候用；前提检查（`asm` 是否可用、版本是否够，不够时怎么装）；命令一览；通用规则（默认排除当前会话、先看大纲再读回合、不要整份读大会话、会话内容不外传）；找会话时读哪份参考文档。缓存检查的场景在 P2 才加入，这一期的 skill 不提它。
- `references/cli.md`：CLI 完整规范，每个命令的参数、默认值、输出格式、JSON 字段、退出码；旧命令也列出，注明给人用和比对用。
- `references/finding-sessions.md`：找会话的方法：拆概念、扩写说法（中英文、错误码、命令名、项目名）、选文本类别、由宽到窄的调整办法、核实、报告格式；至少两个完整的例子，其中一个就是「近期某个 app 连不上网」；常见陷阱（thinking 大多为空、项目名常常不在正文里、`other` 里的 compact 摘要会重复前文）。

开发期间：在本仓库里开 Claude Code 就能用到这个 skill；`make install-cli` 把 `asm` 装到 `~/.local/bin`。

### 不做的事

- 不改 app 的搜索和界面；app 里搜索 User / Assistant 的结果和现在一样（thinking 进了索引，但 app 不搜它）。
- 不做相关度打分，不做同义词词典，扩写说法由 agent 自己完成。
- 不搜 subagent 的对话。
- `list`、`cost`、`stats`、收藏相关命令和旧命令的输出不变（颜色默认值除外）。
- 缓存检查（P2）、分词和向量（P3）、对外发布（P4）都不在这一期。

### 验收标准

- `asm search-old` / `asm session-old` 与改名前输出一致，`make parity` 全部通过。
- 在真实数据上，`asm search "连不上|无法连接|offline"` 按会话列出结果，每个结果有标题、项目、时间、各类别命中数、带回合号的片段和恢复命令，当前会话不在其中；加 `--include-self` 后出现。
- 两个概念分别只出现在同一会话的不同消息里时，该会话被找到；加 `--same-message` 后不再出现。
- `--in thinking` 能找到只在 thinking 里出现的词；`--in tool-output` 能找到只在工具输出里出现的错误信息。
- 最近 30 天、带工具输出的字面搜索，在本机一次完成的耗时有实测记录，目标 2 秒以内。
- `asm session <前缀>` 给出卡片；前缀不唯一时列出候选并以非 0 退出。
- `asm projects --days 30` 列出最近 30 天活跃的项目。
- `asm transcript ID --outline`、`--turns`、`--grep` 输出对应的回合，回合号与 `asm search` 片段里的一致，也与 app 详情里的 Turn 编号一致；不带新选项时输出与现在逐字相同。
- 输出给管道时没有颜色码；`--json` 输出能被 `jq` 解析，字段与 `cli.md` 一致。
- `cli.md` 缺少某个命令或参数时，测试失败。
- 在本仓库新开的 Claude Code 会话里问「帮我找一下近期关于某个 app 连不上网的问题解决会话」，agent 加载 skill，找到相关会话，核实后给出报告和恢复命令。

## Tech Design

### 现状

- CLI 全在 `crates/core/src/bin/asm.rs`（815 行）：clap derive，`Cmd` 枚举；`search` 默认走 `search::regex::search`（扫原始文件、按 mtime 新到旧、rayon 每批 32 个文件、每行 `classify` 后 `serde_json::from_slice::<RawRecord>`），`--index` 走 `search::fts::search_sessions`。颜色只由 `--no-color` / `NO_COLOR` 决定（`main()` 里的 `color`），不看是否终端。`transcript` 子命令和独立的 `transcript-reader` 共用 `src/bin/support/transcript.rs`（`#[path]` 引入）。
- 索引：`schema.sql` 的 `messages`（`role` 只有 user / assistant，`is_meta`）和 `messages_fts`（trigram，`role`、`session_id` 为 UNINDEXED 列，触发器只同步 `is_meta = 0` 的行）。`SCHEMA_VERSION = 4`（`index/db.rs`）。
- 解析：`claude/parse.rs::TranscriptParser`，`on_user` 把任意有文字的 user 记录写成一行 `MessageRow`（`is_human_prompt` 只用于计数），`on_assistant` 只收 `text` block，thinking 不入库。
- 回合编号：`claude/conversation.rs` 用 `transcript::is_human_input` 判断一条 user 记录是否开启新回合（`self.number += 1`）；`transcript.rs` 的导出用同一个判断，但不输出编号。
- FTS 搜索：`search/fts.rs::count_per_session` 每个词是一个 FTS5 短语（隐式 AND，同一条消息内），短于 3 个字符的词改走 `LIKE`；app 的 `crates/app/src/search.rs` 用 `FtsRoles { user, assistant }` 调 `fts::search`。
- 恢复命令：`crates/app/src/actions.rs::resume_command(id, project_path, dir_exists)`。
- 项目：`Db::projects()` 返回 `ProjectInfo { key, path, session_count, last_updated }`。
- CLI 测试：`crates/core/tests/cli.rs` 用 `assert_cmd` 在 fixture 沙盒里跑 `asm`（设 `NO_COLOR=1`）；其中 `search_renders_tool_calls_on_several_lines_and_highlights_with_ansi` 依赖颜色输出。
- 比对：`scripts/compare-with-python.sh` 调 `asm session` / `asm search`。
- 实测（2026-11-14，load 5–6）：CLI 正则扫描约 1 秒/GB，7 天 1.6 GB 2.4 秒，30 天 5.1 GB 6.0 秒；只搜 user + assistant 与全搜一样慢，时间花在逐行解析上。记录里的中文是原始 UTF-8，抽查的文件里没有 `\uXXXX` 转义。

### 方案

新命令和旧命令分开实现：旧命令搬进 `old.rs` 原样保留，新命令共用一层「会话级搜索」。会话级搜索有两条路径，按查询条件自动选：

- **索引路径**：请求的类别都在索引里（`prompt` / `reply` / `thinking` / `other`）且不是 `--regex`。每个概念的每种说法各查一次 FTS（短词走 `LIKE`），只取命中行的 id、session、类别、位置、时间，在 Rust 里按会话求「每个概念都有命中」的交集，最后只给选中的片段取正文。
- **扫描路径**：请求了工具类别，或 `--regex`。所有请求的类别都由扫描完成（不和索引结果混合，片段口径一致）。扫描前先做两级字节预过滤：整份文件里每个概念至少出现一种说法，否则整份跳过；通过的文件再逐行过滤，行里出现任一说法才解析 JSON。

两条路径产出同一个 `SessionHits` 结构，之后的排序、截断、渲染（文字 / JSON）共用。

关键决定：

- **thinking 和 `other` 进索引，用新列 `kind` 区分**，`role` 保留。被否决：把 thinking 当成一个新 role（会改变 `role` 的含义，app 的 `FtsRoles` 要跟着改语义）。
- **`prompt` 与 `other` 的划分和回合编号同一口径**：一条 user 记录是 `prompt` 当且仅当它开启新回合（`transcript::opens_turn`：`is_human_input` 为真且不是中断标记；计划交接的 `Implement the following plan:…` 开回合，本地命令的输出不开，brief 决定 13、24、25）。这样 `asm search` 片段的回合号可以直接从索引算出（同一会话里位置不晚于命中行的 `prompt` 行数），与 `transcript --turns`、app 详情一致。
- **扫描路径不复用旧的 `search::regex::search`**，另写 `scan_groups`，预过滤只加在新路径上；旧命令和 app 的正则模式这一期不动，`make parity` 不受影响。被否决：直接改 `find_in_bytes`（要同时满足 Python 比对和 app 的流式扫描，风险大，留到以后单独做）。
- **时间范围按会话活动时间过滤**：会话的活动区间（首条 prompt 到最后更新）与范围相交就参与（规则见 `cli.md`「Time range」），命中可以在会话任何位置，两条路径口径一致。被否决：按消息时间过滤（会话跨天时，「近期」的会话会因为关键句在范围外被漏掉）。
- **CLI 拆成目录**：`src/bin/asm/main.rs` 加若干模块，避免单文件继续膨胀。

### 数据变动

- `schema.sql`：
  - `messages` 加 `kind TEXT NOT NULL CHECK (kind IN ('prompt','reply','thinking','other'))`；`role` 的 CHECK 不变（thinking 行的 role 是 assistant）。
  - `messages_fts` 加 `kind UNINDEXED` 列，两个触发器同步写入 `kind`。
- `SCHEMA_VERSION` 4 → 5（P1-2 改回合规则后为 6），打开旧索引时整库重建（既有机制，无迁移）。同步 `db.rs` 里 `insert_messages` / `messages()` 的列。
- `model.rs`：新增 `pub enum TextKind { Prompt, Reply, Thinking, Other }`（`as_str` / `parse`），`MessageRow` 加 `kind: TextKind`。
- 一条 assistant 记录可能同时有 thinking 和 text block，会写成两行（相同 `byte_offset`，不同 `kind`）；`messages_session_ts` 不是唯一索引，不受影响。
- 只有 thinking 正文非空（trim 后）的 block 入库；同一记录的多个 thinking block 用换行拼成一行。

### 逻辑变动

**core 解析（`claude/parse.rs`）**

- `RawRecord` 补上 `is_human_input` 需要的字段（`isCompactSummary`、`origin.kind`，如尚未有）。把 `transcript::is_human_input` 的判断改成只依赖一个小的输入结构（或给 `RawRecord` 写一个等价函数），两处共用，避免两套口径。
- `on_user`：`kind = Prompt` 若这条记录开启新回合（`opens_turn`），否则 `Other`；哪些记录被跳过（主记录里的 sidechain、同 uuid 的重复）由 `transcript::RecordFilter` 决定，详情面板、导出、解析器共用，追加扫描时 indexer 把已索引的 user uuid 交给解析器；只贴了图片的输入写一行空文本；`delta` 的计数逻辑不变（`user_msg_count` 仍按 `is_human_prompt`，和 Python 脚本一致）。
- `on_assistant`：text block 照旧拼成 `Reply` 行；另把非空 thinking 拼成一行 `Thinking`。`assistant_msg_count`、`last_message` 只看 text，不变。
- `user_line_may_have_text` 的预过滤不变；`usage_only` 模式不产生消息行，不变。
- `gen_fixtures.py`：加一个含 thinking 正文的 assistant 记录、一个空 thinking（只有签名）、一条 `origin.kind` 非 human 的 user 记录、一条 compact 摘要，`uv run` 重生成并提交。

**core 搜索**

- 新模块 `search/query.rs`：`TermGroup { alternatives: Vec<String> }`、`Query { groups, regex: bool, same_message: bool }`；`Query::parse(terms: &[String], regex)` 按 `|` 切分说法（字面模式下切分；正则模式整个 TERM 是一个正则）；`Query::matcher()` 生成高亮用的 `Matcher`（字面：各说法转义后 `|` 连接，不分大小写；正则：`Matcher::regex(.., false)` 的语义，编不过按字面）。
- 新模块 `search/sessions.rs`：
  - `KindSet`（prompt / reply / thinking / tool_input / tool_output / other，含 `tool` 名字过滤 `tools: Vec<String>`）。
  - `SessionFilter { project_needles, since_ms, until_ms, model, favorites, exclude: Option<String> }`。会话集合先由 `Db::list_sessions` + 过滤得到（主模型用 `cost::primary_model`，同 `asm cost --model`），两条路径都只看这个集合。
  - `SessionHits { session_id, counts: BTreeMap<Kind, u32>, groups_hit: Vec<bool>, snippets: Vec<Hit> }`，`Hit { kind, turn: Option<u32>, ts, byte_offset, tool: Option<String>, text: Highlighted }`。
  - `search_index(db, &Query, &KindSet, &sessions) -> Vec<SessionHits>`：每个说法一次查询，`SELECT m.id, m.session_id, m.kind, m.byte_offset, m.ts FROM messages_fts JOIN messages m … WHERE messages_fts MATCH ?`（短词 `LIKE`，沿用 `fts.rs::needs_like` / `escape_like`），`m.kind IN (…)`、`m.session_id` 在集合里（集合大时用临时表或先查后在 Rust 里过滤，实现时按耗时选）。按会话合并：会话级模式要求每个 group 至少一行；`--same-message` 时按 message id 求交集。片段挑选：命中 group 数多的消息优先，其次按 `prompt > reply > thinking > other`，再按位置；只对选中的消息取 `text` 做 `Matcher::snippet`。回合号：`SELECT count(*) FROM messages WHERE session_id = ? AND kind = 'prompt' AND byte_offset <= ?`（同位置的 prompt 行算进去）。
  - `search_scan(files, &Query, &KindSet, …) -> Vec<SessionHits>`：见下。
- 新模块 `search/prefilter.rs`：
  - 字面模式：每个说法若「可预过滤」就进 aho-corasick（`ascii_case_insensitive`），不可预过滤的说法让它所在的 group 永远放行。可预过滤的条件：不含 `"`、`\`、控制字符；不含有大小写之分的非 ASCII 字母（`c.is_lowercase() || c.is_uppercase()` 且非 ASCII，因为 aho-corasick 只做 ASCII 大小写折叠）；CJK 等无大小写的字符可以。
  - 正则模式：用 `regex-syntax` 的字面量提取，得到的每个字面量都可预过滤且数量不超过 64 时用它们，否则这个 group 放行。
  - 文件级：对整份映射做一次多模式查找，记录每个 group 是否出现；有 group 未出现且它不是「放行」，整份跳过。行级：行内出现任一可预过滤说法、或存在放行 group 时才解析。
  - 依赖：`aho-corasick`、`regex-syntax` 用 `cargo add` 显式加到 core（二者已是 `regex` 的传递依赖）。
- `search_scan` 的扫描：沿用 `discover_all` 列文件（按会话集合过滤，不含 subagent）、`map_file`、rayon 并行（按文件大小排序后分批，避免一批里一个大文件拖住）；每行通过预过滤后解析，复用 `regex.rs::iter_texts` 的拆块方式，把块映射到 `Kind`（user 文本按 `is_human_input` 分 prompt / other；`tool_use` → tool_input，`tool_result` → tool_output，带工具名）。逐行维护当前回合号（遇到 `prompt` 加一），所以片段的回合号不用查索引。`iter_texts` 改为 `pub(crate)` 并返回块的类别，旧路径的行为不变。
- `fts.rs`：`FtsQuery.roles` 的语义保持「app 的 User / Assistant」：内部换成 kind 过滤，User → `prompt, other`，Assistant → `reply`。app 的搜索结果与现在一致（有测试）。

**core 其他**

- `transcript.rs`：新增 `TranscriptOptions { outline, turns: Option<TurnRange>, grep: Option<Query>, context, thinking, tools }` 和 `read_transcript_with(path, &opts)`，实现在 `transcript/reading.rs`。`TurnRange { first, last: Option<u32> }` 支持 `N`、`N-M`、`N-`（原写的 `(u32, u32)` 表达不了 `N-`）。没有阅读选项时直接调 `read_transcript`，输出逐字不变（`tests/transcript_reader.rs` 不改）。
  - 按范围读时每个回合显示 `asm search` 搜的全部 item（不按 `RecordFilter` 丢记录），`<user turn="5" ts="2026-11-13 08:01">` 里是原文；`--outline` 每回合一行，用清理后的首行。
  - `--grep` 判定 prompt、reply、thinking（不加 `--thinking` 也判，命中的 thought 照样显示）和 `other`；工具调用只在 `--tools` 时判定（brief 决定 22）。有命中的回合保留，再按 `--context` 带上前后回合（可以从范围外的命中回合伸进范围）。保留的回合里只显示 prompt、命中的 item 和它前后各 1 个 item，其余折叠成一行 `…[skipped N items, M chars; --turns N without --grep shows them]`；不带 `--grep` 的 `--turns` 照旧整回合（决定 23）。
  - 开头标签的 `matched` 是有命中的回合数，带 `--context` 时另有 `context="N"`。`TranscriptExport.tool_hits_hidden` 数范围内因为命中只在工具调用里、没带 `--tools` 而没保留的回合，CLI 在 stderr 提示加 `--tools`（exit 0）。
  - 工具调用的输入和输出按字符截断：没有命中取前 2,000；有命中取前 500，再给每个命中开窗 `[h-300, h+700)`，相接的窗口合并，累计到 4,000 后这个命中和之后的都不开窗，计入最后一个空隙行 `…[cut N chars, M more hits]`。`<user>`、`<agent>`、`<thinking>` 不截。细节以 `cli.md` 为准。
- 恢复命令：`resume_command` 和 `shell_quote` 从 `crates/app/src/actions.rs` 移到 core（如 `core::format` 或新的 `core::session`），app 改为调用 core，app 的测试不变。

**CLI（`crates/core/src/bin/asm/`）**

- `main.rs`：`Cli`、全局参数（加 `--color`，`--no-color` 作为 `--color never` 的别名）、分发。颜色 `auto` = stdout 是终端且 `NO_COLOR` 为空。
- `old.rs`：原 `list` / `session` / `search` 及其辅助函数原样搬入；子命令名改为 `search-old`、`session-old`。`index`、`list`、`cost`、`stats`、收藏命令、`transcript` 保持原名。
- `search.rs`（新 `search`）、`session.rs`（新 `session`）、`projects.rs`、`render.rs`（文字 / JSON 渲染、`«»` 标记、时间范围头行）。
- 公共参数结构 `RangeArgs { days, since, until }`（`days` 与 `since` 互斥）、`SessionFilterArgs { project, model, favorites, include_self }`。当前会话：读 `CLAUDE_CODE_SESSION_ID`（Claude Code 给子进程设的，2026-11-14 核对过值就是当前会话 id），`--include-self` 时不排除。
- JSON：`serde::Serialize` 的输出结构，顶层 `{ "meta": {…}, "results": […] }`；`meta` 含 `terms`、`kinds`、`range`、`engine`（`index` / `scan`）、`elapsed_ms`、`total`、`shown`、`truncated`。字段名在 `cli.md` 里列全。
- 写锁回退：新命令共用 `open_db`：`Indexer::scan` 返回 SQLite busy / locked 时，丢弃这次扫描，打印 stderr 一行提示，继续查询。旧命令同样适用（行为只在原本会报错时变化）。
- `support/transcript.rs::ReaderArgs` 加上新选项，`transcript-reader` 二进制自动获得。
- 规范一致性测试：在 bin 里 `#[cfg(test)]`，用 `clap::CommandFactory::command()` 遍历子命令与参数，读 `skills/asm/references/cli.md`（`env!("CARGO_MANIFEST_DIR")` 起的相对路径），断言每个子命令有 `### asm <name>` 小节、小节里出现每个 `--long` 参数。
- `crates/core/tests/cli.rs`：依赖颜色的那个测试改传 `--color always`；`asm session` / `asm search` 的旧测试改用 `session-old` / `search-old`。

**脚本与构建**

- `scripts/compare-with-python.sh`：`asm_args=(session …)` → `session-old`，`(search …)` → `search-old`。
- `Makefile`：新增 `install-cli`（release 构建后 `install -m 755` 复制到 `$(PREFIX)/bin/asm`，`PREFIX ?= $(HOME)/.local`）；`help` 文案更新。复制而不是 `ln -sf`（brief 决定 26）：开发中的构建会反复重建共享构建目录里的 `asm` 并提升 `SCHEMA_VERSION`，符号链接会让全局 `asm` 悄悄变成开发版；代价是更新后要重跑。
- 仓库加 `.claude/skills/asm` → `../../skills/asm` 的相对符号链接（确认 `.gitignore` 没有忽略 `.claude/`）。

**skill（`skills/asm/`，英文）**

- `SKILL.md` frontmatter：`name: asm`；`description` 写清触发场景（find / search past Claude Code sessions or conversations, "where did we fix X", "找一下之前……的会话", resume an old session, read what happened in a session）。正文：前提检查（`asm projects --help`：`asm --version` 恒为 `0.1.0`，旧版也有 `search`，都分不出新旧；缺失或旧版时在本仓库 `make install-cli`）、命令一览、通用规则、场景路由（P1 只有找会话）。
- `references/cli.md`：每个命令一个 `### asm <name>` 小节：用途、参数表（名字、取值、默认值）、输出示例、JSON 字段、退出码（0 成功，含 0 结果；1 错误；2 参数错误）。
- `references/finding-sessions.md`：方法与例子（PRD 第 7 点）。例子里给出真实可跑的命令序列和期望看到的输出形状。

### 实施步骤

- [x] 1. 写 `skills/asm/references/cli.md` 的 P1 部分（新 `search` / `session` / `projects` / `transcript` 选项 / 全局参数 / 旧命令一览）和规范一致性测试；此时测试应失败
  - 2026-11-16 完成（P1-1，与第 2、9 步一起）：`kb/sessions/2026-11-15-asm-p1-cli.md`
- [x] 2. CLI 拆目录、旧命令改名、`--color`；更新 `tests/cli.rs` 和 `compare-with-python.sh`；`cargo test -p agent-session-core` 和 `make parity` 通过
  - 2026-11-16 完成：`kb/sessions/2026-11-15-asm-p1-cli.md`
- [x] 3. `TextKind`、schema 5、解析出 thinking / prompt / other；先补 fixture 和测试（类别划分、全量 == 增量、回合号与 `read_conversation` 一致、app `FtsRoles` 结果不变），再实现
  - 2026-11-16 完成（P1-2，含决定 24、25 的回合规则，schema 6）：`kb/sessions/2026-11-15-asm-p1-kinds.md`
- [x] 4. `search/query.rs` + `search/sessions.rs::search_index`：先写行为测试（会话级 AND、`|`、`--same-message`、类别过滤、短词、时间与项目过滤、排除会话、片段回合号）
  - 2026-11-16 完成（P1-3，与第 5 步一起）：`kb/sessions/2026-11-15-asm-p1-search.md`
- [x] 5. `search/prefilter.rs` + `search_scan`：先写测试（JSON 转义、引号 / 反斜杠 / 换行、大小写、非 ASCII、正则字面量提取、预过滤与不过滤结果相同），再实现；在真实数据上量 30 天带工具输出的耗时
  - 2026-11-16 完成：`kb/sessions/2026-11-15-asm-p1-search.md`（扫描在 `search/scan.rs`）
- [x] 6. 新 `asm search`（文字 / JSON、默认时间、排序、截断、空结果提示、排除当前会话）+ CLI 测试
  - 2026-11-16 完成（P1-4，与第 7 步一起）：`kb/sessions/2026-11-16-asm-p1-cmds.md`
- [x] 7. `resume_command` 移到 core；新 `asm session`、`asm projects` + 测试
  - 2026-11-16 完成：`kb/sessions/2026-11-16-asm-p1-cmds.md`（移到 core `session.rs`）
- [x] 8. `transcript` 新选项 + 测试（默认输出不变、回合号一致、`--grep` / `--context` / `--tools` 截断）
  - 2026-11-16 完成（P1-5）：`kb/sessions/2026-11-16-asm-p1-transcript.md`
- [x] 9. 写锁回退（用一个持有写事务的连接模拟 app）+ 测试
  - 2026-11-16 完成（并入 P1-1）：`kb/sessions/2026-11-15-asm-p1-cli.md`
- [x] 10. `SKILL.md`、`finding-sessions.md`、cli.md 补全；`.claude/skills/asm` 链接、`make install-cli`
  - 2026-11-17 完成（P1-6）：`kb/sessions/2026-11-16-asm-p1-skill.md`
- [x] 11. 真实数据验收（见下），新开 Claude Code 会话验收 skill
  - 2026-11-17 完成（P1-7）：`kb/sessions/2026-11-17-asm-p1-accept.md`
- [x] 12. 更新 `AGENTS.md`（当前状态、开发命令、parity 说明）、`kb/docs/product-prd.md`、`README.md`，commit
  - 2026-11-17 完成：README 与 product-prd 在 P1-6，其余在 P1 收尾：`kb/sessions/2026-11-17-asm-p1-docs.md`

### 测试与验证

| 验收标准 | 怎么验 |
|---|---|
| 旧命令改名后输出不变 | `tests/cli.rs` 旧用例改名后通过；`make parity` 全部 PASS |
| 会话级结果、片段、恢复命令、排除当前会话 | `tests/cli.rs` 新用例（fixture 沙盒，设 `CLAUDE_CODE_SESSION_ID` 为某个 fixture 会话）；真实数据上 `asm search "连不上\|无法连接\|offline"` 人工核对 |
| 跨消息 AND / `--same-message` | `search/sessions.rs` 行为测试 + fixture |
| thinking / 工具输出可搜 | fixture 里只在 thinking、只在 tool_result 出现的词各一个 |
| 30 天带工具输出 ≤ 2 秒 | 真实数据实测 3 次取中位数，记进 session 总结；超出时记下瓶颈，不阻塞验收但要写进 `known-issues.md`。实测：P1-3 在 30 天 5.7 GB 上中位数 0.86–2.0 s；P1-7 验收时负载 3.5–6.5，`--in tool` 的 `connection refused` 2.49 s、`ECONNRESET` 1.92 s、`offline` 1.75 s，在目标边缘，没做空载复测，已记进 `known-issues.md` |
| `session` 卡片 / 前缀不唯一 | CLI 测试 |
| `projects --days` | CLI 测试 |
| transcript 选项、回合号一致 | `tests/transcript_reader.rs` 新用例；对真实数据的 3 个会话比较 `asm transcript --outline` 的回合数与 `asm session-old --conversation` 的 `turns=` |
| 无颜色码、JSON 可解析 | CLI 测试断言输出不含 `\x1b`；`--json` 输出用 `serde_json` 解析并检查字段 |
| cli.md 漂移 | 规范一致性测试；临时删掉一个参数的说明，确认失败 |
| skill 端到端 | 在本仓库新开 Claude Code 会话（herdr 新 tab）提问，记录调用的命令序列和最终回答，存进 `tmp/<date>-asm-p1/` |

### 风险与待定

- `is_human_input` 依赖 `origin`、`isCompactSummary` 等字段，`parse.rs` 的 `RawRecord` 和 `transcript::Record` 是两个结构；若共用判断改动过大，退一步写两份并用测试在 fixture 和真实数据上断言一致。
- 常见词（如 `error`）在索引路径会返回大量行。先量：只取 id 等小列应在 100ms 内；超出时改为每个 group 先求会话集合、再只对候选会话取行。
- 预过滤假设记录里非 ASCII 字符不转义。第 5 步先在全部真实记录上统计 `\u` 转义的出现情况（预期只有控制字符），不成立时把非 ASCII 说法标为不可预过滤。
- 扫描路径的耗时目标依赖「大部分文件整份跳过」。概念里有很常见的说法（如 `error`）时跳不掉，耗时接近 1 秒/GB；`finding-sessions.md` 要提醒 agent 搜工具输出时尽量用具体的错误信息并配合时间范围。
