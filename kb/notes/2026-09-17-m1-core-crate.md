---
created: 2026-09-17
tags:
  - m1
  - core
  - parser
  - sqlite
  - fts5
  - search
  - asm
  - agent-session-manager
---

# M1 记录：核心 crate（解析器、SQLite 索引、搜索、`asm` CLI）

PRD 里程碑 M1 完成。`crates/core`（`agent-session-core`）从空壳变成完整的数据层，不依赖 gpui；`asm` CLI 复刻了 `~/Code/scripts/claude_session_search.py` 的三种模式并在本机真实数据上做了逐字比对。本文记录模块划分、与设计文档的差异、实测数字和验收方法。

## 模块

```text
crates/core/src
├── model.rs              SessionSummary（= sessions 表一行）、MessageRow、Role、Source、标题优先级常量
├── format.rs             human_size / fmt_datetime / fmt_time / parse_ts（与 Python 输出一致）
├── claude/
│   ├── paths.rs          ClaudeDir（ASM_CLAUDE_DIR > CLAUDE_CONFIG_DIR > ~/.claude）、encode_project_path、default_db_path
│   ├── discover.rs       顶层 <project>/<session>.jsonl 枚举 + stat；subagent 只计数
│   ├── clean.rs          clean_prompt（去 <system-reminder> 等包裹块、/cmd args 还原）、one_line
│   ├── parse.rs          TranscriptParser：feed(bytes, offset) → MessageRow + SessionDelta；classify() 字节预过滤
│   └── history.rs        history.jsonl 加载、pasted 展开（内联 / paste-cache）、2 小时间隔匿名分组
├── index/
│   ├── schema.sql        projects / sessions / session_tool_counts / session_files / messages / messages_fts / index_state
│   ├── db.rs             Db：open（损坏即删、版本不符即重建）、upsert、list_sessions、find_by_prefix、stats
│   └── indexer.rs        Indexer::scan / rebuild：stat → plan（Unchanged / Append / Full / Rebuild）→ rayon 解析 → 单事务写入 → 标记缺失 → history 补位
├── search/
│   ├── fts.rs            FTS5 trigram 查询构造、<3 字 token 走 LIKE、snippet 标记转高亮区间、按 session 聚合
│   ├── regex.rs          扫原始文件：iter_texts / make_snippet / render_tool_use / format_command(shfmt) 的移植，分批并行 + limit 提前停止
│   └── pyjson.rs         Python json.dumps 风格序列化（`, ` / `: `），保证 tool_use 输入文本与脚本一致
└── bin/asm.rs            asm index | list | session | search [--index] | stats
```

测试在 `crates/core/tests/`（61 个场景，BDD 先写后实现）：`clean_prompt` / `parse_transcript` / `history` / `index` / `search_regex` / `search_fts` / `cli`。fixture 是一棵假 `~/.claude`（`tests/fixtures/claude/`），由 `tests/fixtures/gen_fixtures.py` 生成（`uv run` 即可重生成），覆盖全部已知记录类型：isMeta、命令标签、图片载体、sidechain、tool_result 数组形态、attachment、cost-state、四种标题记录、pr-link、history 匿名分组、粘贴占位符三种解析结果、非法 JSON 行。测试把 fixture 拷到临时目录并固定 mtime，CLI 测试固定 `TZ=UTC` + `NO_COLOR=1`。

## 关键实现细节

- **字节预过滤**（`parse.rs::classify`）：assistant 记录的顶层 `"type":"assistant"` 排在整个 `message` 之后（可能几 MB 之后），attachment 记录的第一个 `"type"` 在内层对象里。所以规则是：前 512 字节含 `"attachment":` → 跳过；否则取第一个 `"type":` 的值，是 `message` 就再找 `"role":`。在本机 194,031 行真实记录上验证：没有一行找不到 type / role。tool_result-only 的 user 记录（语料 321MB）用 `"content":[` 且不含 `"type":"text"` 判定后整行跳过，不做 JSON 解析。
- **增量语义**：`SessionDelta::apply` 让"全量一次解析"与"多次追加解析"结果相同（有测试断言相等）："first" 字段只设一次，计数累加，"last" 字段覆盖，标题按 custom > agent-name > ai-title > summary 的 rank 覆盖。半行（live session 正在写）不消费，`indexed_offset` 停在最后一个换行。
- **文件状态机**：`(size, mtime_ns)` 未变 → 跳过；`size < indexed_offset` 或 inode 变化 → Rebuild（删 messages 后从 0 解析）；否则从 `indexed_offset` 追加（前一字节必须是 `\n`，否则也 Rebuild）；`index_state` 有记录但文件不在 → `transcript_missing = 1`，保留元数据，删 messages。
- **history 补位**：每次扫描重读 history.jsonl（3.8MB，130ms），`session_sources()` 一次拿到所有 id → source，只 upsert 非 transcript 来源的 session。匿名 session 的 id 是 `history:<project>:<first_ts>`，`id_known = 0`，CLI 显示 `(unknown session id)`。
- **prompt 计数与 Python 完全一致**：`isMeta`、sidechain、`[Image: source:` 载体、tool_result-only 不算；清理后为空的 prompt 仍计数但不作为 first prompt；history 模式每行算一条，含斜杠命令。
- **RawValue 限制**：serde_json 的 `&RawValue` 不能出现在 `#[serde(tag)]` / `untagged` / `flatten` 之下（内部先缓冲成 Content 再分发），所以记录和 block 都是"一个大结构体 + `Option` 字段 + 按 `type` 字段 match"，content 的 string / array 分支靠首字节判断后分别反序列化。
- **regex 模式对齐脚本**：一个 piece（文本块 / thinking / tool_use 输入 / tool_result 输出）最多算一次命中；上下文按字符数截取，`⏎` 代换行；tool_use 渲染 `# description` / `$ command`（超过 80 列时 shfmt -i 2）/ `k: v`，超过 300 字符的字段围绕命中截断。文件按 32 个一批 rayon 并行，批间检查 limit，所以 `N matches in M sessions scanned (showing first L)` 的数字和顺序执行的脚本一致。

## 与设计文档 / PRD 的差异

| 设计 | 实际 | 原因 |
|---|---|---|
| `grep-searcher` + `grep-regex` 扫文件 | memmap2 + memchr 切行 + `regex` crate | 匹配在 JSON 解码后的文本上做（转义、`k: v` 拼接都让原始行匹配不可靠），grep-searcher 的行级搜索没用武之地；少 5 个依赖 |
| `jiff` | `chrono` | app crate 已经用 chrono 格式化本地时间，避免两套时间库；chrono 读 `TZ` 让 CLI 测试可固定时区 |
| `history_prompts` 表 | 没有 | history session 直接物化进 `sessions`（source = history），列表不需要单独的 prompt 表；M5 做详情面板的"history-only prompt 列表"时再加 |
| subagent 消息进 `messages`（is_sidechain = 1，默认不进 FTS） | 只统计 `subagent_count` | R9 排在 M5；主文件里的 sidechain 记录仍按原样进 messages（脚本搜索也包含它们） |
| `sessions-index.json` 作标题候选 | 未读 | R8，M5 |
| `snippet(…, 24)` | `snippet(…, 64)` | trigram 下一个 token 只有一个字符的位移，24 token ≈ 26 字符太短；64 是 FTS5 上限 |
| `messages_session_ts(session_id, ts)` | `(session_id, byte_offset)` | 会话顺序 = 文件顺序；ts 有 0（无时间戳记录）的情况 |

## 实测（M4 系列 Mac，release 构建，本机 840 份主 transcript / 1.1GB / 216 个项目 / 11,790 行 history）

| 场景 | 耗时 |
|---|---|
| 全量索引（冷，`asm index --rebuild`） | 2.1–2.4s：解析 0.07–0.13s，写 SQLite 1.8–2.0s，history 0.13s |
| 其中 FTS5 trigram 建索引（14.5MB 文本，`INSERT INTO messages_fts(messages_fts) VALUES('rebuild')` 单测） | 1.6s（固有成本） |
| 增量无变化扫描（840 次 stat + history 重读） | 0.14s |
| `asm list` 含自动增量扫描 | 0.14s |
| 正则扫全部 1.1GB（热） | 0.35–0.47s；脚本同条件 30 天范围 2.1s，asm 0.22s |
| FTS 查询（`--index grafana` 85 命中 / `重构` LIKE 路径 105 命中） | 3ms / 11ms |
| 索引体积 | 63MB（5,384 sessions，其中 4,544 仅 history；11,217 messages） |

写库 2s 里 1.6s 是 trigram 分词，解析本身只有 0.1s。对 M2 的启示：sessions 表写完就可以出表格（约 0.4s），FTS 可以放在其后异步做；或者 rebuild 时先写 messages 再一次性 `rebuild` FTS（比触发器逐行快约 20%）。

## 验收

`scripts/compare-with-python.sh [ASM_BIN] [OUT_DIR]`（`make parity`）在真实 `~/.claude` 上跑 21 组命令，去掉 ANSI 后 `diff -u`：7 组 `-L`（子串、路径、`--full`、`-d 30`、跨项目 limit）、3 组 `-s`、11 组搜索（角色、标签、大小写、上下文、非法正则回退、中文、`-p`、limit）。2026-09-17 全部 PASS，输出归档在 `tmp/2026-09-17-python-parity/`。

已知的行为边界（脚本比不出来，但要记住）：

- 正则方言是 Rust `regex`（无回溯、无 lookaround）；Python 能编译而 Rust 不能的模式会退化为字面量匹配。
- transcript 被清理后，`asm list` 仍显示索引里保存的元数据（prompt 数、标题）并标 `[history only]`；脚本只能显示 history 里的信息。全新索引时两者一致。
- `-L PATH` 中 PATH 若恰好是当前目录下存在的子目录（如仓库里的 `scripts/`），两者都按目录解析并报错，要写 `Code-scripts` 这类子串。
