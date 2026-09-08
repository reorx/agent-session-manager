---
created: 2026-09-08
tags:
  - data-layer
  - sqlite
  - fts5
  - indexing
  - rust
  - research
---

# 数据层设计：索引与搜索 Claude Code Transcript

调研日期 2026-09-08。本文给出解析、索引、搜索、增量更新的技术选型与数据模型，是 `kb/docs/claude-session-data-format.md`（数据格式）的下游。所有数字均在本机（M4，macOS 26.6，Claude Code 2.1.260）实测。

## 1. 决定设计的关键实测

| 指标 | 值 |
|---|---|
| 项目目录 / JSONL 文件 | 85 / 896（654 主 transcript + 242 subagent，subagent 共 97MB） |
| 总大小 / 行数 | 917MB / 148,611 行，平均一行 5.7KB，**最长一行 1.35MB** |
| 各类内容字节占比 | `toolUseResult`（顶层重复）381MB、`tool_result` block 321MB、`tool_use.input` 30MB、**user 文本 12MB、assistant 文本 6.6MB、thinking 0.1MB** |
| **可检索的人类 / 助手文本** | **≈18.5MB，占语料 2%** |
| ripgrep 全量扫描（页缓存热，8 线程） | 固定串 0.10s；正则 0.06s；`-i` 中文 0.06s；`"type":"user".*重构` 0.12s |
| `cat` 全部文件（热） | 0.17s |
| Python `json.loads` 全量 | 2.1–2.6s（≈400MB/s；Rust 解析器 3–6 倍） |
| `settings.json` | `cleanupPeriodDays: 60` |

**结论**：全文索引只需覆盖约 20MB 文本，FTS5 的体积、构建时间、查询延迟都不是问题；900MB 的难点纯粹是"解析时跳过 98% 无用字节"，而不是搜索。thinking 在新版 transcript 里几乎为空，不要围绕它设计功能。

## 2. 存储 / 索引引擎选型

### 2.1 SQLite（rusqlite bundled）+ FTS5 —— 采用

`rusqlite = { version = "0.40", features = ["bundled"] }` 内置 SQLite 3.53 且默认开启 `SQLITE_ENABLE_FTS5`，无需加载扩展。**单个 `Connection` 由一个后台线程独占**（closure 通道或 `Arc<Mutex<Connection>>`），桌面应用一个写者足够；WAL 模式下如需"导入期间读不阻塞"再开第二个只读连接。

```rust
let conn = Connection::open(db_path)?;
conn.execute_batch(r#"
  PRAGMA journal_mode = WAL;
  PRAGMA synchronous = NORMAL;
  PRAGMA temp_store = MEMORY;
  PRAGMA mmap_size = 268435456;
  PRAGMA cache_size = -65536;    -- 64 MB
  PRAGMA foreign_keys = ON;
"#)?;
```

FTS5 用 **external-content** 表挂在 `messages` 上，文本只存一份：

```sql
CREATE VIRTUAL TABLE messages_fts USING fts5(
  text,
  role UNINDEXED, session_id UNINDEXED,
  content='messages', content_rowid='id',
  tokenize = "trigram case_sensitive 0"
);
CREATE TRIGGER messages_ai AFTER INSERT ON messages WHEN new.is_meta = 0 BEGIN
  INSERT INTO messages_fts(rowid, text, role, session_id) VALUES (new.id, new.text, new.role, new.session_id);
END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, text, role, session_id) VALUES ('delete', old.id, old.text, old.role, old.session_id);
END;
-- 消息不可变，无 UPDATE trigger；重建 = delete + insert
```

带角色、项目、时间范围过滤的查询：

```sql
SELECT m.id, m.session_id, m.uuid, m.role, m.ts,
       snippet(messages_fts, 0, char(1), char(2), '…', 24) AS snip,
       bm25(messages_fts) AS score
FROM messages_fts
JOIN messages m ON m.id = messages_fts.rowid
JOIN sessions s ON s.id = m.session_id
WHERE messages_fts MATCH ?1
  AND (?2 IS NULL OR m.role = ?2)          -- 'user' | 'assistant'
  AND (?3 IS NULL OR s.project_id = ?3)
  AND m.ts BETWEEN ?4 AND ?5
ORDER BY CASE WHEN ?6 = 'recency' THEN m.ts END DESC, score
LIMIT 200;
```

注意：`role` / `session_id` 过滤放在**基表列**上而不是 FTS 列过滤（UNINDEXED 列不能进 MATCH）；FTS5 先返回 rowid 再 join 很快。所有语句 `prepare_cached`。

**FTS5 查询语法陷阱**：`- @ . : ( *` 及 `AND/OR/NOT/NEAR` 都是操作符，**用户输入绝不能直接传入**。规范化为短语：按空白拆词，每个词用双引号包住（内部 `"` 双写），空格连接（隐式 AND）；前缀匹配 `*` 只能放在引号外（`"foo"*` 合法，`"foo*"` 不合法）。`ORDER BY rank` 等价 `bm25()`，越小越好。

```rust
fn to_fts_query(input: &str, prefix: bool) -> String {
    input.split_whitespace()
        .map(|t| { let q = format!("\"{}\"", t.replace('"', "\"\"")); if prefix { q + "*" } else { q } })
        .collect::<Vec<_>>().join(" ")
}
```

体积估算：20MB 文本 → trigram 索引 20–40MB（unicode61 约 0.3–0.6 倍）；`messages` 表 20MB；总 DB **50–80MB**。

### 2.2 tantivy —— 不采用

优点：<10ms 打开、BM25、`RegexQuery`（基于 token 的 fst 正则，无 `\b` / 懒惰量词）、CJK 可用 `tantivy-jieba`。缺点：不可变 segment（更新 = 删除 + 重建 + commit + reader reload）、独立目录、元数据仍要 SQLite、多 10–20 个 crate。对 20MB 语料是杀鸡用牛刀。

### 2.3 ripgrep 式原始扫描 —— 作为正则模式与 tool 输出搜索的补充

实测热扫描 917MB 仅 60–120ms，冷启动受 NVMe 读速限制约 0.3–0.5s。**不适合做主引擎**：每次按键烧 8 核、拉 900MB 过缓存；无排名/摘要/角色过滤；语料每几个月翻倍。但作为"正则模式"和"搜索 tool 输出"的按需路径非常好。`claude-view` 项目（Rust/Axum）就是 SQLite 元数据预过滤 + `grep-searcher` 扫原始文件，1,493 个 session <500ms 启动、~50MB 内存。

```rust
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{SearcherBuilder, sinks::UTF8, MmapChoice};
let matcher = RegexMatcherBuilder::new().case_insensitive(true).build(pattern)?;
let mut searcher = SearcherBuilder::new().line_number(false)
    .memory_map(unsafe { MmapChoice::auto() }).build();
searcher.search_path(&matcher, path, UTF8(|_, line| {
    // 先用字节前缀判断 "type":"user" / "assistant" 再解析 JSON
    hits.push(parse_min(line)?); Ok(true)
}))?;
// 用 rayon 对 SQLite 预过滤（项目/时间）后的候选文件并行执行
```

### 2.4 redb / sled / fjall —— 不采用

没有 FTS、没有即席过滤排序、没有 `snippet()`；SQLite 已能做 KV 缓存，再加一种文件格式毫无收益。

## 3. JSONL 解析性能

语料 98% 是索引不需要的 tool 输出，胜负手在于**不物化它们**：

1. **切行**：`memmap2::Mmap` + `memchr::memchr_iter(b'\n', &mmap)`（NEON 下 >10GB/s）。文件只增长，每次 mmap 新的 `len`。
2. **JSON 前的字节预过滤**：每条记录以 `{"type":"…"` 或 `{"parentUuid":…` 开头，用 `memchr::memmem::Finder` 在前 ~200 字节找 `"type":"user"` / `"type":"assistant"` / 元数据类型，`attachment`（27k 条、体积巨大）、`file-history-*`、`mode`、`atis-latch`、`permission-mode` 按前缀直接跳过，不解析。
3. **`serde_json` 部分反序列化**：`#[serde(borrow)] Cow<'a, str>`（无转义时借用，有 `\n` `\"` 时才分配）、`&'a RawValue` 承接 `message.content` 与 `toolUseResult`，300MB+ 的 tool 输出永不拷贝；`content` 再二次解析成 `Vec<Block<'a>>`，其中 `tool_result.content` 仍是 `RawValue`（只量字节长度）。**不要加 `deny_unknown_fields`**，schema 每月都在加字段（user 一种记录就见到 25+ 可选键）。
4. **并行**：`rayon::par_iter` 遍历文件（896 个单元，10 核吃得饱），每个 worker 返回 `Vec<MessageRow>`；SQLite 线程每文件一个事务写入（`prepare_cached` INSERT，10–30 万行/秒）。
5. **预期**：`serde_json` 在此数据上 0.5–1GB/s；`sonic-rs` 在 aarch64 快 1.5–2 倍但要 `target-cpu=native` 且 unsafe；`simd-json` 需要 `&mut [u8]`，与只读 mmap 不兼容。预过滤跳过约 60% 行、`RawValue` 跳过约 98% 字节后，**冷全量索引 917MB 预计 1–2s（读盘瓶颈），热 ~0.5s**。先用 serde_json + RawValue，profile 后再考虑 sonic-rs。

## 4. 增量索引

`index_state` 每文件一行：`(path PK, size, mtime_ns, inode, indexed_offset, last_uuid, schema_version)`。

每次扫描（启动与 watcher 事件）：

- `stat` 每个 `*.jsonl`（896 次约 5ms）。`(size, mtime)` 未变 → 跳过。
- `size > indexed_offset` 且前段未变（Claude Code 只追加；廉价校验 `byte[indexed_offset-1] == b'\n'`）→ mmap 后只解析 `indexed_offset` 到**最后一个换行符**；live session 正在写的半行留到下次。聚合字段用 `UPDATE sessions SET n = n + ?` 增量更新。
- `size < indexed_offset` 或 inode 变化 → 文件被重写（compact、`--resume` fork、`/rewind`）→ `DELETE FROM messages WHERE session_id = ?`（trigger 清 FTS）后从 0 重解析。
- 文件消失 → `sessions.transcript_missing = 1`，行保留；`history.jsonl` 里同 `sessionId` 的行成为 prompt 的回退来源。`history.jsonl` 也是 append-only，同样按偏移增量导入，键 `(sessionId, timestamp)`。
- subagent 文件（`<session>/subagents/agent-*.jsonl`）带 `agentId`、`isSidechain: true`、有时 `sessionKind: "bg"`：作为 `messages` 索引，`parent_session_id` = 外层目录 uuid，`is_sidechain = 1`，**默认不进 FTS**（Task agent 的内部对话），提供开关。
- 全量重建触发：`PRAGMA user_version != SCHEMA_VERSION`、tokenizer 变更、菜单 "Rebuild index" → DROP 全部、重建、重扫。

实时更新：`notify 8.2` + `notify-debouncer-full 0.7`（FSEvents 后端），`new_debouncer(Duration::from_millis(500), None, cb)`，监听 `~/.claude/projects`（递归）与 `~/.claude/history.jsonl`。FSEvents 会把追加合并成 `Modify(Data)`；**不要信任事件类型**（Create/Modify/Remove 顺序不可靠），每批事件只对触碰到的路径重新 `stat` 走上面的状态机。另加 30s 轮询兜底（FSEvents 高负载下会丢事件）。

## 5. 搜索 UX 相关能力

- **摘要与高亮**：`snippet(messages_fts, 0, char(1), char(2), '…', 24)` 返回带控制字符标记的文本，Rust 侧切分为 GPUI `StyledText` 的高亮区间；`highlight()` 给整条消息。正则路径用 `regex::Regex::find_iter` 直接得字节偏移。
- **排序**：默认 `bm25()` 相关度；提供"最新优先"切换；可在 SQL 里做轻混合 `bm25 - 0.1 * age_days`。
- **大小写**：trigram 默认不敏感（`case_sensitive 0`），unicode61 也折叠；正则模式用 `(?i)`。
- **CJK，真正的取舍**：`unicode61` 把连续汉字当一个 token，`MATCH '重构'` 匹配不到 `代码重构一下`；`trigram` 支持任意子串，**但查询短于 3 个码点返回空**（两字中文词：人事、重构）。方案：
  1. **首发采用**：trigram FTS + Rust 侧长度判断，<3 字时改走 `SELECT … FROM messages WHERE text LIKE '%重构%'`（对内容表而非 FTS 表），20MB 线性扫描约 10–20ms。简单且正确。
  2. 索引时用 `jieba-rs` 预分词进独立 `text_seg` 列（unicode61），查询同样分词。中文 BM25 质量更好，但词典加载约 100ms，分词错误变成漏检。
  3. `sqlite-better-trigram` 自定义 tokenizer（C，处理 <3 字）——失去 LIKE 加速且需自定义构建。
- **正则模式**：用户切换 `.*` 开关 → `grep-searcher` 扫 SQLite 预过滤后的文件，rayon 并行，结果流式进 UI。也是搜索 tool 输出的唯一途径。

## 6. 索引存放位置与启动路径

`directories::ProjectDirs::from("dev", "reorx", "AgentSessionManager").data_dir()` → `~/Library/Application Support/AgentSessionManager/index.sqlite`（+ `-wal`、`-shm`）。

启动：打开 DB → 检查 `PRAGMA user_version` → schema 匹配则立即 `SELECT` session 列表（几百到几千行，<5ms）绘制 UI，然后后台线程跑 stat 扫描并把增量推给 view；schema 过期显示"Rebuilding index…"并重建（冷 1–2s）。首次运行等同重建。

## 7. 数据模型

### 7.1 SQL DDL

```sql
PRAGMA user_version = 1;

CREATE TABLE projects (
  id INTEGER PRIMARY KEY,
  encoded_dir TEXT NOT NULL UNIQUE,   -- '-Users-reorx-Code-scripts'
  path TEXT,                          -- '/Users/reorx/Code/scripts'，来自 cwd / history.project / sessions-index.originalPath
  display_name TEXT,
  agent TEXT NOT NULL DEFAULT 'claude-code'   -- 预留 codex 等
);

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,                -- sessionId uuid
  project_id INTEGER NOT NULL REFERENCES projects(id),
  file_path TEXT NOT NULL,
  parent_session_id TEXT,             -- subagent 文件指向外层 session
  is_sidechain INTEGER NOT NULL DEFAULT 0,
  title TEXT,                         -- custom-title > agent-name > ai-title > sessions-index.summary > legacy summary > first_prompt[..80]
  first_prompt TEXT, last_prompt TEXT, last_message TEXT, last_role TEXT,
  started_at INTEGER, updated_at INTEGER,       -- unix ms
  git_branch TEXT, cwd TEXT, cc_version TEXT, entrypoint TEXT, permission_mode TEXT,
  is_background INTEGER NOT NULL DEFAULT 0,
  user_msg_count INTEGER DEFAULT 0, assistant_msg_count INTEGER DEFAULT 0,
  tool_use_count INTEGER DEFAULT 0, tool_result_bytes INTEGER DEFAULT 0,
  subagent_count INTEGER DEFAULT 0,
  file_size INTEGER DEFAULT 0,
  total_cost_usd REAL, duration_ms INTEGER, lines_added INTEGER, lines_removed INTEGER,  -- cost-state
  pr_url TEXT,
  source TEXT NOT NULL DEFAULT 'transcript',    -- 'transcript' | 'history'
  transcript_missing INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX sessions_updated ON sessions(updated_at DESC);
CREATE INDEX sessions_project ON sessions(project_id, updated_at DESC);

CREATE TABLE session_tool_counts (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  tool_name TEXT NOT NULL, count INTEGER NOT NULL,
  PRIMARY KEY (session_id, tool_name)
) WITHOUT ROWID;

CREATE TABLE session_files (          -- file-history-delta.trackingPath 去重
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  path TEXT NOT NULL,
  PRIMARY KEY (session_id, path)
) WITHOUT ROWID;

CREATE TABLE messages (
  id INTEGER PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  uuid TEXT NOT NULL, parent_uuid TEXT,
  role TEXT NOT NULL CHECK (role IN ('user','assistant')),
  ts INTEGER NOT NULL,
  text TEXT NOT NULL,                 -- 拼接的 text block（不含 thinking / tool_result）
  is_meta INTEGER NOT NULL DEFAULT 0, -- harness 注入的 user 文本；trigger 排除出 FTS
  byte_offset INTEGER NOT NULL, byte_len INTEGER NOT NULL,   -- 按需回读原始行
  model TEXT, input_tokens INTEGER, output_tokens INTEGER,
  UNIQUE (session_id, uuid)
);
CREATE INDEX messages_session_ts ON messages(session_id, ts);

CREATE VIRTUAL TABLE messages_fts USING fts5(
  text, role UNINDEXED, session_id UNINDEXED,
  content='messages', content_rowid='id', tokenize="trigram case_sensitive 0");
-- triggers 见 §2.1

CREATE TABLE index_state (
  path TEXT PRIMARY KEY, size INTEGER, mtime_ns INTEGER, inode INTEGER,
  indexed_offset INTEGER NOT NULL DEFAULT 0, last_uuid TEXT, indexed_at INTEGER
);

CREATE TABLE history_prompts (        -- ~/.claude/history.jsonl 回退
  session_id TEXT, ts INTEGER, project TEXT, display TEXT,
  PRIMARY KEY (session_id, ts)
) WITHOUT ROWID;
```

### 7.2 Rust 记录类型（serde，借用）

```rust
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum Record<'a> {
    User { #[serde(flatten, borrow)] env: Envelope<'a>, #[serde(borrow)] message: Msg<'a>,
           #[serde(default, rename = "isMeta")] is_meta: bool,
           #[serde(default, rename = "isCompactSummary")] is_compact_summary: bool },
    Assistant { #[serde(flatten, borrow)] env: Envelope<'a>, #[serde(borrow)] message: Msg<'a> },
    System { #[serde(flatten, borrow)] env: Envelope<'a>, subtype: &'a str,
             #[serde(rename = "durationMs")] duration_ms: Option<u64> },
    Summary { summary: Cow<'a, str>, #[serde(rename = "leafUuid")] leaf_uuid: &'a str },   // 旧版
    LastPrompt { #[serde(rename = "lastPrompt", borrow)] last_prompt: Option<Cow<'a, str>> },
    CustomTitle { #[serde(rename = "customTitle", borrow)] title: Cow<'a, str> },
    AiTitle     { #[serde(rename = "aiTitle", borrow)]     title: Cow<'a, str> },
    AgentName   { #[serde(rename = "agentName", borrow)]   name:  Cow<'a, str> },
    CostState   { #[serde(rename = "totalCostUSD")] total_cost_usd: f64,
                  #[serde(rename = "totalDuration")] total_duration: u64,
                  #[serde(rename = "totalLinesAdded")] lines_added: u64,
                  #[serde(rename = "totalLinesRemoved")] lines_removed: u64 },
    PrLink { #[serde(rename = "prUrl")] pr_url: &'a str },
    #[serde(other)] Other,   // attachment, mode, atis-latch, file-history-*, queue-operation, frame-link, 未来类型
}

#[derive(Deserialize)]
struct Envelope<'a> {
    uuid: &'a str,
    #[serde(rename = "parentUuid")] parent_uuid: Option<&'a str>,
    timestamp: &'a str,                              // RFC3339，用 jiff / time 解析
    #[serde(rename = "sessionId")] session_id: &'a str,
    cwd: Option<&'a str>,
    #[serde(rename = "gitBranch")] git_branch: Option<&'a str>,
    version: Option<&'a str>,
    #[serde(default, rename = "isSidechain")] is_sidechain: bool,
    #[serde(rename = "agentId")] agent_id: Option<&'a str>,
    entrypoint: Option<&'a str>,
    #[serde(rename = "sessionKind")] session_kind: Option<&'a str>,
}

#[derive(Deserialize)]
struct Msg<'a> {
    role: &'a str,
    #[serde(borrow)] content: &'a RawValue,          // 懒解析成 Content<'a>
    model: Option<&'a str>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Content<'a> { Text(#[serde(borrow)] Cow<'a, str>), Blocks(#[serde(borrow)] Vec<Block<'a>>) }

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block<'a> {
    Text { #[serde(borrow)] text: Cow<'a, str> },
    Thinking { #[serde(borrow)] thinking: Cow<'a, str> },
    ToolUse { id: &'a str, name: &'a str, #[serde(borrow)] input: &'a RawValue },
    ToolResult { #[serde(rename = "tool_use_id")] tool_use_id: &'a str,
                 #[serde(borrow)] content: Option<&'a RawValue>, #[serde(default)] is_error: bool },
    Image {},
    #[serde(other)] Other,
}
```

`#[serde(other)]` 要求 unit variant，因此未来新增的 `type` 不会打断索引。

## 8. 先例与竞品

| 项目 | 技术 | 特点 |
|---|---|---|
| [tombelieber/claude-view](https://github.com/tombelieber/claude-view) | Rust/Axum + React | SQLite 元数据 + ripgrep-core 扫原始 JSONL；多 session 实时监控；<500ms 启动。**架构上最接近的兄弟项目** |
| [jhlee0409/claude-code-history-viewer](https://github.com/jhlee0409/claude-code-history-viewer) | Tauri v2 | 直接读 JSONL（mmap + memchr 取元数据行），全局搜索 |
| [EthanJags/claude-sessions-viewer](https://github.com/EthanJags/claude-sessions-viewer) | Swift 原生 macOS | 按项目浏览，复制 `claude --resume` |
| [delexw/claude-code-trace](https://github.com/delexw/claude-code-trace) | Rust + Node | 桌面/网页/TUI，live tail |
| [d-kimuson/claude-code-viewer](https://github.com/d-kimuson/claude-code-viewer) | Web | 实时、Cmd-K 全文搜索、可启动 session |
| [kunwar-shah/claudex](https://github.com/kunwar-shah/claudex)、[drewburchfield/claude-code-chat-explorer](https://github.com/drewburchfield/claude-code-chat-explorer) | SQLite/FTS5 web | claudex 还暴露 MCP 工具 |
| [raine/claude-history](https://github.com/raine/claude-history) | TUI | fuzzy / lexical / semantic 搜索，resume / fork / rename |
| [daaain/claude-code-log](https://github.com/daaain/claude-code-log)、[annenpolka/cclog](https://github.com/annenpolka/cclog)、ccusage | 转换 / 统计 | HTML/MD 导出、token 报表 |
| `~/Code/agent-sessions`（本人旧项目） | Python SSG | Claude Code + Codex 统一中间格式后渲染成静态站点，可作 Codex adapter 参考 |

Schema 参考：[anthropics/claude-code#53516](https://github.com/anthropics/claude-code/issues/53516)（要求官方文档化 schema，列出各行类型）、[maxturazzini/claude-viewer DATASOURCES.md](https://github.com/maxturazzini/claude-viewer/blob/main/DATASOURCES.md)。

## 9. 最终选型

1. **SQLite（rusqlite bundled，WAL，单写线程）是全部持久层**：`projects / sessions / messages / index_state / history_prompts` + `messages_fts`（FTS5 external content，trigram，`case_sensitive 0`）。只索引 user/assistant 文本（今天约 20MB），永不索引 tool 输出、attachment、thinking。预期 DB 50–80MB，冷构建 <2s，查询约 10ms。
2. **解析**：`memmap2` + `memchr` 切行，字节级类型预过滤，`serde_json` + `RawValue` / `Cow` 借用，`rayon` 按文件并行，单写线程每文件一事务。profile 显示解析瓶颈时再考虑 `sonic-rs`。
3. **增量**：`(size, mtime, inode, indexed_offset)` 尾部解析；重写检测 → 单 session 重建；`notify-debouncer-full` + 30s 轮询；`history.jsonl` 与 `sessions-index.json` 作为被清理 transcript 的回退。
4. **搜索模式**：FTS5（bm25 或最新优先、`snippet()` 高亮、角色/项目/时间在基表过滤）；1–2 字 CJK 查询走 `LIKE`；正则模式 `grep-searcher` 扫 SQLite 预过滤后的文件（也是搜 tool 输出的唯一路径）。
5. 索引放 `~/Library/Application Support/AgentSessionManager/index.sqlite`；`PRAGMA user_version` 门控全量重建。

**推测性提醒**：transcript schema 无官方文档且每月变化（`summary` 已被 `last-prompt` / `ai-title` 取代；`atis-latch`、`cost-state`、`frame-link` 都是新增），每个字段都当可选、每个未知 `type` 都当可忽略。

来源：[SQLite FTS5](https://www.sqlite.org/fts5.html) · [rusqlite](https://github.com/rusqlite/rusqlite) · [tantivy](https://github.com/quickwit-oss/tantivy) · [grep-searcher](https://docs.rs/grep-searcher/latest/grep_searcher/) · [notify-debouncer-full](https://docs.rs/notify-debouncer-full/) · [sonic-rs](https://github.com/cloudwego/sonic-rs) · [FTS5 trigram CJK 混合方案](https://zenn.dev/kanseilink/articles/kanseilink-fts5-trigram-cjk-20260507?locale=en) · [Turso: Beyond FTS5](https://turso.tech/blog/beyond-fts5)
