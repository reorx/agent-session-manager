---
created: 2026-11-16
tags:
  - usage
  - ledger
  - sqlite
  - indexing
  - agent-session-manager
  - plan
---

# 用量账本：读到过的用量永久保留，状态栏显示今天的费用

总 plan 是 [index.md](index.md)，共用口径（4:00 分日、按调用时间归属、In 的定义、只增不删）在那里。这是第 1 期，不依赖其他期。

## PRD

### 背景

用量现在只存在索引里。索引可以随时从磁盘上的记录文件重建，重建时记录已被删掉的 session 的用量就丢了。会触发重建的事很常见：改价格表、改解析、索引结构变化、菜单里的 Rebuild Index、索引文件损坏、数据目录变化。两个月里索引的版本号从 1 涨到了 5。

应用也只在启动和手动 Scan for Changes 时读新数据，所以「今天」的数字总是停在上一次扫描的时刻。

### 目标

应用读到过的每一次 API 调用都记进一份单独的账本，之后无论删记录文件、重建索引还是索引损坏，账本里的数字都不变。应用开着时自己定时读新数据，状态栏随时能看到今天的费用。

### 功能与交互

**账本**

- 账本是数据目录里的一个单独文件，和索引分开。应用和 `asm` 命令行读到新的用量时都会记进去，两边记的是同一份。
- 账本记三样东西：每次 API 调用的模型、时间和各类 token 数；每个 session 的项目路径、标题、类型（CLI / SDK / 后台）和起止时间；项目的路径。**不存对话内容。** 标题没有时用首条 prompt 的第一行，最多 200 个字符。
- 账本长期保留 session 标题和项目路径，即使对应的记录文件已经删掉。这是这份功能的目的，用户需要知道这一点；账本文件只在本机，不上传。
- 账本只增不删：一次调用记进去之后，记录文件被删、subagent 的记录被单独删、文件被重写，账本里都还在。
- 不提供查看、编辑、清空账本的界面。

**第一次启动**

- 升级到带账本的版本后第一次启动，应用把当前索引里的全部用量抄进账本，包括记录文件已经被删、但索引里还留着用量的 session。这一步约 1 秒，只发生一次，期间表格晚 1 秒出现。
- 如果这次升级同时需要重建索引，抄写发生在重建之前，旧索引里的用量不会丢。

**索引出事时**

- Rebuild Index、索引版本变化、数据目录变化引起的自动重建：账本不动。重建后会话表里已删的 session 可能消失或只剩 prompt，它们的用量仍在账本里。
- 索引文件损坏：应用照旧删掉索引重建，账本不动。
- 账本文件自己损坏：应用把损坏的文件改名留在原目录（不删除），新建一份账本，把当前索引里的用量抄进去。索引里已经没有的历史这时找不回来，损坏的文件留着供以后手动处理。
- 账本是更新版本的应用写的（降级运行旧版本）：这次运行不读也不写账本，其余功能照常；状态栏的今天费用显示 `—`。换回新版本后，这段时间漏记的用量从索引补上。

**一天从 4:00 开始**

- 状态栏的今天，以及会话表 Updated 下拉的全部预设（Today / 3 days / 7 days / 30 days / This month / Last month / Custom），都按 4:00 分日。例如凌晨 2 点选 Today，看到的是前一天 4:00 以来更新过的 session；Custom 选 11-01 到 11-03，范围是 11-01 4:00 到 11-04 4:00。
- 自定义范围的日期选择器里，「今天」和快捷项（本月、上月、近 7 天、近 30 天）也按这条规则。
- 应用连续开着跨过 4:00 时，Today 这类预设和状态栏的今天自动换到新的一天，不需要重启或重新选一次。
- 表格里 Started / Updated 列显示的日期时间不变，仍是日历时间。

**状态栏的今天**

- 状态栏左侧在 session 数和合计后面加上今天的费用：`42 / 5488 sessions · $123.45 · Today $203.69`。这是今天（4:00 起）全部调用的费用，不受表格的过滤和搜索影响，隐藏的项目和没有 prompt 的 session 也算在内。
- 每次扫描结束后更新。今天还没有调用时显示 `Today $0.00`；账本读不了时显示 `Today —`。
- 今天有调用用了价格表里没有的模型时，后面加 `(N unpriced)`，N 是这类调用的次数。
- 这一期它只是一个数字，点击没有反应；点击进入 Usage 视图由 [usage-view](usage-view.md) 加上。

**自动扫描**

- 应用开着时，在这些时刻自动做一次增量扫描：启动时（现有行为）、窗口回到前台时、之后每 10 分钟。菜单里的 Scan for Changes 照旧。
- 自动扫描是安静的：没有变化时界面没有任何反应，状态栏中间照旧显示上一次扫描的摘要。上一次扫描还没结束，或者结束不到 30 秒，就跳过这一次。
- 扫描发现变化时，会话表和手动扫描一样整表刷新：正在进行的 session 的 Updated 变新，按默认排序会排到前面；选中的 session 保持选中，详情面板不重新加载；正在搜索时搜索会重跑一次。
- 另一个进程（`asm`）正好占着索引时，这一次自动扫描放弃，不报错，等下一次。

**命令行**

- `asm` 的各个命令在查询前会扫描，扫到的新用量同样记进账本。`asm index --rebuild` 不动账本。
- 这一期不加查看账本的命令。

### 不做的事

- 不做 Usage 视图、图表、菜单栏（后面三期）。
- 不做文件监听；自动扫描是定时的，一次约 1 秒，大部分时间花在检查文件有没有变。
- 不改会话表 Cost 列和状态栏合计的口径，也不把会话表的费用改成从账本读。
- 不做账本的导出、清空、合并，不做从备份恢复的界面。
- 不做 4:00 的设置项。

### 验收标准

1. 增量扫描读到新调用后，账本里有这些调用、它们所属 session 的信息和小时汇总；同一次调用被两次扫描各读到一半，账本里只有一行。
2. 删掉一批记录文件后扫描，再 Rebuild Index，账本里每个 session 的调用数和各类 token 数都和删之前一样；状态栏的今天费用不变。
3. session 还在、它的 subagent 记录被单独删掉：会话表里这个 session 的费用下降，账本里的调用不减少。
4. 索引版本和当前版本不符时启动：旧索引里的用量先进了账本，然后索引才被重建；旧索引太老、没有用量表时，重建照常完成，不报错。
5. 索引文件损坏时启动：索引被删掉重建，账本文件原样保留。
6. 账本文件损坏时启动：原文件改名留在目录里，新账本里有当前索引的全部用量。
7. 账本里缺了几行索引里有的调用（模拟崩溃）：下一次打开后补齐。
8. 带旧索引（没有账本）第一次启动：索引里全部 session 的用量进了账本，包括记录已删的。
9. 改价格表里某个模型的单价后，同一份账本算出的费用按新单价变化；没有价格的模型费用为 0，并计入 unpriced 次数。
10. 应用的索引和 `asm` 的索引在同一个目录各自扫描同一批文件，账本里每次调用只有一行。
11. 状态栏显示 `Today $X`，X 等于账本里今天 4:00 以来全部调用的费用；扫到新调用后更新；过了 4:00 归零重计。
12. 会话表的 Today / 3 days / 7 days / 30 days / This month / Last month / Custom 都以 4:00 为界；凌晨 4 点前「今天」是前一个日历日。
13. 往一个记录文件追加内容后不做任何操作，10 分钟内表格里这一行更新；让窗口失去再得到焦点，立即触发一次扫描；30 秒内连续触发只扫一次。
14. 降级：账本版本比当前版本新时，应用正常启动、会话表可用，账本文件没有被改动。

## Tech Design

### 现状

- `crates/core/src/index/db.rs`：`Db` 持有一个连接。`open_with` 调 `open_checked`，遇到 `is_corrupt` 的错误就 `remove_db_files` 再开一次。`open_checked` 依次 `use_wal` → `configure` → `ensure_schema`；`ensure_schema` 发现 `schema_mismatch` 就进 `build_schema`，在一个 `BEGIN IMMEDIATE` 事务里 `reset_schema`（删掉 main 库的全部表再建）。写 spec 时的基线是 5；core 实现开始时 `bd56682` 已提交，`SCHEMA_VERSION` 为 6，本期保持 6。实测用的备份实际为版本 4，消息表也与当前版本不同。
- 用量的写入：`insert_usage`（按 `(session_id, agent, msg_id)` upsert，`ts` 取较小值）、`delete_usage`、`refresh_session_usage`（重写 `session_model_usage` 和 `sessions` 上的用量列）。`clear_all` 清空包括 `api_usage` 在内的全部表。`Db::transaction` 用 `BEGIN IMMEDIATE` 包住一次扫描的全部写入。
- `crates/core/src/index/indexer.rs`：`scan_streaming` 在一个 `db.transaction` 里依次 `forget_subagents`、`write_subagent`、`write_parsed`、`mark_missing`、`import_history`。会让索引丢用量的地方：`rebuild` → `clear_all`；`write_parsed` / `write_subagent` 在非追加时先 `delete_usage`；`forget_subagents` 在 session 自己的文件还在时删掉 subagent 的用量；`reset_schema`；`remove_db_files`。`mark_missing` 保留用量。`check_roots` 发现根目录变化时返回 `RootsChanged`，app 的 `Worker::scan` 收到后带 `rebuild = true` 重来。
- `crates/core/src/claude/paths.rs`：`default_db_path()`（`$ASM_DB`，否则数据目录的 `index.sqlite`）、`default_cli_db_path()`（`$ASM_DB`，否则同目录的 `index-cli.sqlite`）。`asm` 经 `App::open_index` → `Db::open_with` 打开，`open_db_refreshed` 在查询前扫描。
- `crates/app/src/store.rs`：`Worker::open` 打开 `Db`，`run` 先 `load` 再 `scan(false)`，然后处理 `StoreRequest`（`Scan`、`Rebuild` 等）。`Worker::scan` 把 `ScanEvent` 转成 `StoreEvent::Progress` / `Session`，结束发 `ScanDone`，有变化再 `load`。
- `crates/app/src/main.rs`：`AppRoot` 拥有 `Store`；`on_store_events` 成批应用事件；`rescan` 把 `index_status` 设成 `Loading` 后发请求；`render_status_bar` 的左侧是 `{shown} / {total} sessions · {cost}{projection}`。没有定时器，也不监听窗口激活。
- `crates/app/src/model.rs`：`RangePreset::bounds` 取 `Local::now().date_naive()` 调 `bounds_on(today, custom)`，各预设用 `local_midnight` 把日期换成毫秒。边界只在 `AppRoot::new` 和 `toolbar.rs::set_range` / `set_custom_range` 时算一次，之后一直用同一组数字。`toolbar.rs::date_presets` 和 `RangePreset::default_custom` 也取日历上的今天。

### 方案

core 新增 `ledger.rs`：`Ledger` 自己持有一个 SQLite 连接，文件是索引同目录的 `usage-ledger.sqlite`，有自己的版本号和迁移。`Db` 在打开时一并打开它，`insert_usage`、`refresh_session_usage`、`upsert_project` 把要记的东西攒在内存里，`Db::transaction` 在提交索引之前用一个短事务把它们写进账本并提交。`Db` 每次打开都按 session 比较索引和账本的调用数，把账本缺的从索引补上；第一次回填、崩溃后的补漏、账本重建后的回填都是这一步。索引需要重建时，在 `reset_schema` 之前先把旧索引的用量整个抄进账本。

core 新增 `usage.rs`：分日规则，和内存里的用量数据 `UsageData`（按 session 存小时汇总，能算任意时间段的合计）。app 的 store 线程启动时把账本的小时汇总整表发给界面，之后每次扫描只发有变化的 session；`AppRoot` 据此算今天的费用显示在状态栏。`AppRoot` 加一个 10 分钟的定时器和窗口激活的监听来触发扫描。

关键决定：

- **账本用单独的连接，不用 `ATTACH`**（决定表第 10 条举的例子是 `ATTACH`）。`ATTACH` 的两个 WAL 库在一次提交里不保证跨文件原子，SQLite 按库的顺序先提交 main，崩溃会留下「索引提交了、账本没有」，而增量扫描不会再送这些行。两个连接可以固定先提交账本：崩溃只会留下「账本有、索引没有」，索引的 `index_state` 没前进，下次扫描重读同样的字节，upsert 的结果相同。另外账本作为独立的类型可以单独测试，损坏时的处理也不牵连索引的连接。代价是回填不能写成一条跨库的 `INSERT … SELECT`，要在 Rust 里读出再写（一次性，约 1 秒）。
- **账本的写入先攒着，在索引提交前一次写完。** 备选是账本事务和索引事务同时开、同时持有。那样一次冷扫描（3–4 秒）期间账本一直被锁着，而 `index.sqlite` 和 `index-cli.sqlite` 共用一份账本，app 和 `asm` 会互相等，2026-11-16 把两个索引分开就是为了让它们不互相等。攒着写，账本的锁只持有写入那一下（增量扫描几毫秒，冷扫描约半秒）。
- **用「比调用数」做核对，不用「已回填」标记。** 每次打开比较每个 session 在索引里的调用数（`session_model_usage.responses` 之和）和在账本里的调用数（`usage_hourly.responses` 之和），两边都是几千行的小表，约 2–5 ms。它同时覆盖第一次回填、崩溃补漏、账本重建、旧版本应用写过索引、用备份换回索引这几种情况。直接比 `api_usage` 和 `responses` 两张大表要 38 ms，而且随历史线性变慢（一年约 0.35 秒），`asm` 每条命令都要付这个代价。
- **索引重建前的抢救是整表抄，不走核对。** 旧版本的索引可能没有 `session_model_usage`（版本 4 才有），核对用不了。抢救只在重建前发生一次，后面跟着 3–4 秒的冷扫描，多 1 秒可以接受。旧索引缺表或缺列时这一步跳过，重建照常。
- **session 信息只从记录文件来的行写进账本。** 重建之后，记录已删的 session 可能从 `history.jsonl` 回到索引，那一行没有入口类型、起始时间也不同；如果 `upsert_session` 每次都同步到账本，账本里原本准确的信息会被它盖掉。所以只在 `refresh_session_usage` 时同步，而它只在记录文件有变化时被调用。
- **账本版本比当前版本新时，这次运行不用账本。** 备选是打开失败，那样降级后整个应用不可用。不用账本期间漏记的用量，新版本下次打开时由核对补上。
- **账本不绑定数据目录。** 索引的 `claude_root` 变了会自动重建，账本不管这个：换了数据目录，两边的用量都留在账本里（钱都花了）。session id 是 UUID，不会撞。
- **今天的费用放进状态栏在这一期做**，而不是等 Usage 视图。它让这一期在真实应用里有看得见的验收点，也让 menubar-resident 不必依赖 usage-view。
- **分日规则和 `RangePreset` 的修改也在这一期。** 第一个「今天」的数字出现时，全应用就该只有一种分日方式。
- **边界随时间变。** 应用以后会连续开好几天，`RangePreset` 的边界不能只在选的时候算一次。自动扫描的定时器每次醒来看用量日有没有变，变了就重新应用时间过滤并重算今天。

### 数据变动

新文件 `usage-ledger.sqlite`（WAL，`synchronous = NORMAL`），schema 放在 `crates/core/src/ledger.sql`：

```sql
-- 一次 API 调用一行。主键和各列同索引的 api_usage，没有 cost_usd。只增不删。
CREATE TABLE responses (
  session_id TEXT NOT NULL,
  agent TEXT NOT NULL DEFAULT '',       -- '' 是 session 自己的记录文件，否则是 subagent 文件名
  msg_id TEXT NOT NULL,
  model TEXT NOT NULL,                  -- 原样存，计价时再 normalize
  ts INTEGER NOT NULL,                  -- unix ms
  input INTEGER NOT NULL,
  cache_write_5m INTEGER NOT NULL,
  cache_write_1h INTEGER NOT NULL,
  cache_read INTEGER NOT NULL,
  output INTEGER NOT NULL,
  web_search INTEGER NOT NULL,
  fast INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (session_id, agent, msg_id)
) WITHOUT ROWID;

-- responses 按决定价格的键和小时分组，由 Ledger::refresh_hourly 按 session 重写。
CREATE TABLE usage_hourly (
  session_id TEXT NOT NULL,
  hour INTEGER NOT NULL,                -- ts / 3_600_000，UTC 整点
  model TEXT NOT NULL,
  fast INTEGER NOT NULL,
  long_prompt INTEGER NOT NULL,         -- 这次请求的输入超过 pricing::LONG_PROMPT
  sub INTEGER NOT NULL,                 -- 1 = 来自 subagent 文件
  responses INTEGER NOT NULL,
  input INTEGER NOT NULL,
  cache_write_5m INTEGER NOT NULL,
  cache_write_1h INTEGER NOT NULL,
  cache_read INTEGER NOT NULL,
  output INTEGER NOT NULL,
  web_search INTEGER NOT NULL,
  PRIMARY KEY (session_id, hour, model, fast, long_prompt, sub)
) WITHOUT ROWID;

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  project_key TEXT NOT NULL,            -- 索引里的 projects.encoded_dir
  title TEXT NOT NULL DEFAULT '',       -- display_title() 的前 200 个字符
  entrypoint TEXT,
  is_background INTEGER NOT NULL DEFAULT 0,
  started_at INTEGER,
  updated_at INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;

CREATE TABLE projects (key TEXT PRIMARY KEY, path TEXT) WITHOUT ROWID;

CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
```

- 版本号是账本文件自己的 `PRAGMA user_version`，常量 `ledger::LEDGER_VERSION = 1`，和索引的 `SCHEMA_VERSION` 无关。
- **迁移**：`Ledger::open` 读 `user_version`。0 且没有表：建 schema，写版本号。小于 `LEDGER_VERSION`：按版本逐步执行迁移（每一步一个事务，做完写新的版本号）。等于：不动。大于：返回「版本太新」，`Db` 这次不带账本。以后改结构只能加一步迁移，不能删了重建；`usage_hourly` 是派生表，迁移里可以整表重算。
- `sub` 这一期的界面用不到（主线与 subagent 的拆分不在首版），留在键里是因为它由 `agent` 决定、不占多少空间，以后要用时不必迁移。
- 索引的 schema 不变，`SCHEMA_VERSION` 不提升。
- `state.json` 不变。

### 逻辑变动

**core**

- `claude/paths.rs`：`ledger_path(index: &Path) -> PathBuf`，返回 `index.with_file_name("usage-ledger.sqlite")`。
- `model.rs`：`HourlyUsage { hour: i64, model: String, fast: bool, long_prompt: bool, sub: bool, responses: u64, tokens: Tokens }`；`LedgerSession { id, project_key, project_path: Option<String>, title, entrypoint: Option<String>, is_background: bool, started_at: Option<i64>, updated_at: i64 }`；`SessionUsage { session: LedgerSession, hours: Vec<HourlyUsage> }`。
- `ledger.rs`（新，`lib.rs` 里 `pub mod ledger`）：
  - `Ledger::open(path)`、`Ledger::open_in_memory()`。`open` 设 WAL 和 `busy_timeout`，按上面的规则建表或迁移。打开或读版本号时遇到 `NotADatabase` / `DatabaseCorrupt`：把文件连同 `-wal`、`-shm` 改名为 `usage-ledger.corrupt-<YYYYMMDD-HHMMSS>.sqlite`（及对应后缀），再新建。改名用 `std::fs::rename`，不删除任何文件。
  - `write(&self, batch: &LedgerBatch, index: &Connection)`：一个 `BEGIN IMMEDIATE` 事务里 upsert `batch.responses`（冲突时各列取新值，`ts` 取较小值，和 `Db::insert_usage` 一样）、upsert `batch.projects`（`path = COALESCE(excluded.path, projects.path)`）、对 `batch.sessions` 里的每个 id 从 `index` 读 session 行并 upsert 进 `sessions`（只读 `source = 'transcript'` 的行，读不到就跳过）、`refresh_hourly`。
  - `refresh_hourly(session_id)`：删掉这个 session 的 `usage_hourly`，再从 `responses` 按 `ts / 3600000, model, fast, 输入合计 > LONG_PROMPT, agent <> ''` 分组插入。
  - `response_counts() -> HashMap<String, u64>`（`usage_hourly` 按 session 求 `responses` 之和）。
  - `usage(ids: Option<&[String]>) -> Vec<SessionUsage>`：整表或指定的 session，`sessions` 左连 `projects` 取路径；账本里有调用但没有 session 信息的也返回（标题为空、`project_key` 为空串）。
  - `LedgerBatch { responses: Vec<(String, String, UsageRow)>, projects: Vec<(String, Option<String>)>, sessions: BTreeSet<String> }`。
- `index/db.rs`：
  - `Db` 加 `ledger: Option<Ledger>` 和 `pending: RefCell<LedgerBatch>`。
  - `open_checked`：`configure` 之后、`ensure_schema` 之前打开账本（`Ledger::open(&ledger_path(path))`）；「版本太新」时 `ledger = None` 并 `eprintln!` 一行。`open_in_memory` 配 `Ledger::open_in_memory()`；`open_read_only` 不带账本。
  - `build_schema`：在事务里确认仍然 `schema_mismatch` 之后、`reset_schema` 之前调 `salvage_into_ledger(source: &Connection)`，`source` 传自己的连接：从它读 `api_usage` 的 12 列和 `sessions` 连 `projects` 的信息，整个写进账本并提交。来源做成参数，是为了能把一份索引备份抄进账本（实施步骤 7）。`prepare` 报「no such table / no such column」时跳过对应的部分，不算错误；新建的空索引（版本 0、没有表）直接跳过。读旧索引时遇到损坏类错误：账本的事务回滚，错误照常返回，由 `open_with` 按索引损坏处理。
  - **账本的错误不能被 `open_with` 当成索引损坏。** `is_corrupt` 沿错误链找 `rusqlite` 的错误码，账本连接抛出的 `DatabaseCorrupt` 如果原样带出 `open_checked`，`open_with` 会去删索引。所以账本的损坏一律在 `Ledger` / `flush_ledger` 内部处理掉（改名、新建、补齐），不向外返回；账本的其他错误转成不带 `rusqlite::Error` 的 `anyhow` 错误（把原错误的文字放进消息）再返回。
  - `ensure_schema` 之后调 `reconcile_ledger`：索引侧 `SELECT session_id, sum(responses) FROM session_model_usage GROUP BY 1`，账本侧 `response_counts()`；账本数较小的 session，读出它在 `api_usage` 的全部行，连同它的 id 组成一个 `LedgerBatch` 写进去。
  - `insert_usage`：照旧写索引，再把行推进 `pending.responses`。`refresh_session_usage`：照旧，再把 id 记进 `pending.sessions`。`upsert_project`：照旧，再推进 `pending.projects`。三者末尾：如果 `self.conn.is_autocommit()`（不在事务里），立即 `flush_ledger`。
  - `Db::transaction`：`f` 成功后先 `flush_ledger()`，再提交索引；`f` 失败时清空 `pending`。
  - `flush_ledger`：取出 `pending`，为空则返回；调 `ledger.write`。遇到损坏类错误：让 `Ledger` 改名重建，然后 `reconcile_ledger`（这时索引事务里未提交的行对同一个连接可见），再继续提交索引。其他错误原样返回，索引事务回滚。
  - `delete_usage`、`delete_session`、`clear_all`、`mark_transcript_missing`、`reset_schema`、`remove_db_files` 不碰账本。在 `remove_db_files` 上面加一句注释说明账本文件不在它的列表里是有意的。
  - `Db::ledger_usage(ids: Option<&[String]>) -> Result<Option<Vec<SessionUsage>>>`：没有账本时 `None`。
- `usage.rs`（新）：
  - `DAY_START_HOUR: u32 = 4`。
  - `usage_day<Tz: TimeZone>(ts_ms: i64, tz: &Tz) -> NaiveDate`：换成本地墙上时间（`naive_local()`），减 4 小时，取日期；不要从带时区的瞬间减四个实际小时，否则春季夏令时切换当天 04:00 会落到前一天。
  - `day_start_ms<Tz: TimeZone>(day: NaiveDate, tz: &Tz) -> i64`：`day` 的本地 4:00；夏令时让这一刻出现两次时取较早的，不存在时取下一个存在的时刻。
  - `UsageData`：`BTreeMap<String, SessionUsage>`。`replace_all(Vec<SessionUsage>)`、`apply(Vec<SessionUsage>)`（按 id 替换）。
  - `Totals { responses: u64, tokens: Tokens, cost: CostLines, unpriced_responses: u64 }`；`UsageData::totals(from_ms, until_ms) -> Totals`：桶的 `hour` 落在 `[from_ms / 3600000, until_ms / 3600000)` 的都算；每个桶用 `pricing::price(&model)` 和 `price_tokens(.., fast, long_prompt)` 计价，没有价格的把 `responses` 加进 `unpriced_responses`。计价的入口是 `totals_with(from_ms, until_ms, price: impl Fn(&str) -> Option<ModelPrice>)`，`totals` 传 `pricing::price`；测试传另一张价格表，验证费用确实是从 token 现算的。（按项目、类型、模型的过滤和分组在 usage-view 加。）
- **core 实现补充（2026-11-16）**：`UsageData` 是带公开 `sessions: BTreeMap<String, SessionUsage>` 的结构体；`Ledger::open` 返回 `Result<Ledger>`，新版账本用可识别的 `NewerVersion` 错误表示。账本连接放在 `RefCell<Option<Ledger>>`，另用 `ledger_required` 区分主动禁用和恢复失败；恢复失败后继续报错，不能静默停止记账。使用内部可变连接，以便 `&self` 的写入/查询入口遇到损坏时关闭旧连接、改名、新建并回填。来源索引的元数据先读完，再开始账本事务；仅账本连接的错误带 `LedgerFailure` 标记，避免把来源索引的损坏与账本损坏混为一谈。独立调用 `refresh_hourly` 也有事务保护。
- **Today 的上界**：`totals` 仍严格按上文的 UTC 小时半开区间计算；直接传 `now` 会排除当前小时。app 的 `refresh_today` 必须传当前小时的下一个 UTC 整点，下面的调用示例已修正，否则刚追加的调用要等到下一小时才显示。日期范围的整点上界不变。
- `bin/asm/`：不用改。`Db::open_with` 已经带上账本，`default_cli_db_path()` 和 app 的索引同目录。

**app**

- `store.rs`：
  - `StoreEvent::Usage { sessions: Vec<SessionUsage>, full: bool }`；`StoreEvent::UsageUnavailable`（这次运行没有账本）。
  - `Worker::load_usage()`：`db.ledger_usage(None)`，发 `Usage { full: true }` 或 `UsageUnavailable`。`run` 里在第一次 `load()` 之后调一次。
  - `Worker::scan`：`scan_streaming` 的回调里把每个 `ScanEvent::Session` 的 id 收进一个 `Vec`；扫描成功后、发 `ScanDone` 之前，`db.ledger_usage(Some(&ids))`，非空就发 `Usage { full: false }`。
  - `StoreRequest::Scan` 拆成 `Scan { quiet: bool }`；`quiet` 的扫描遇到 `is_busy` 的错误时不发 `Failed`，发一条 `ScanSkipped`。
- `main.rs`：
  - `AppRoot` 加 `usage: UsageData`、`usage_available: bool`、`today: Totals`、`usage_day: NaiveDate`、`scan_in_flight: bool`、`last_scan_done: Option<Instant>`、`_auto_scan: Task<()>`。
  - `on_store_events`：`Usage { full: true }` → `replace_all`，`full: false` → `apply`；之后 `refresh_today()`（`usage.totals(day_start_ms(usage_day), (now / 3_600_000 + 1) * 3_600_000)`）。`ScanDone` / `Failed` / `ScanSkipped` 清 `scan_in_flight` 并记时间。
  - `auto_scan(cx)`：`scan_in_flight` 或距上次结束不到 `AUTO_SCAN_MIN_GAP`（30 秒）就返回；否则发 `Scan { quiet: true }`，不改 `index_status`。`rescan`（菜单）发 `Scan { quiet: false }`，照旧设 `Loading`。
  - `AppRoot::new`：`cx.observe_window_activation(window, ..)`，窗口变为激活时调 `auto_scan`；起一个循环任务，每 `AUTO_SCAN_IDLE`（600 秒）醒一次，先 `roll_day(cx)` 再 `auto_scan(cx)`。
  - `roll_day(cx)`：`usage_day(now)` 和记着的不同就更新，调 `set_range(self.range, cx)` 让时间过滤重算边界，再 `refresh_today()`。
  - `render_status_bar`：左侧末尾加 `· Today {fmt_usd}`，有 unpriced 时加 `(N unpriced)`，`usage_available` 为假时是 `Today —`。字符串在 `refresh_today` 里格式化好存成字段，render 只取。
- `model.rs`：`RangePreset::bounds` 改为取 `usage_day(now, &Local)` 作为今天；`bounds_on` 里的 `local_midnight` 全部换成 `day_start_ms(.., &Local)`，删掉 `local_midnight`。`default_custom` 的调用方（`AppRoot::new`）和 `toolbar.rs::date_presets` 的调用方改传 `usage_day(now)`。
- **app 实现补充（2026-11-16，usage-p1-app）**：
  - `RangePreset::bounds` 删掉了，不是改写：调用方（`AppRoot::new`、`set_range`）都传 `AppRoot::usage_day` 给 `bounds_on`，`date_presets` 也传它。这样时间过滤、日期选择器和 Today 用同一个「今天」，`roll_day` 注入的时刻也对过滤生效；`model::today_at(now_ms)` 包一层 `usage_day(.., &Local)`。
  - `StoreEvent::ScanDone` 多了 `quiet: bool`。界面靠它做到「自动扫描没有变化时状态栏中间留着上一次的摘要」：只看报告分不清这次是自动的还是菜单发起的（菜单的照旧写 `index up to date`）。
  - `usage_available` 是 `Option<bool>`：`None` 是还没收到账本，状态栏不显示 Today 一段，避免启动时先闪一下 `Today —`。启动时读账本出错：store 线程发 `UsageUnavailable` 和 `Failed`，扫描照常进行。
  - `roll_day(now)` 末尾总是调 `refresh_today(now)`，收到 `Usage` / `ScanDone`、窗口激活、定时器醒来都会调它；所以「今天」只有 `roll_day` 一个入口，Today 不会在换日之前按旧的一天算。
  - `auto_scan(trigger, cx)` 多一个触发来源参数，只用于日志（`auto-scan: timer` / `window activated`），返回是否发出了请求；计时用 `cx.background_executor().now()`（测试能用 `advance_clock` 推进）。菜单的 `rescan` 也置 `scan_in_flight`；`scan_in_flight` 初值为 true（启动扫描）。
  - store 侧的请求处理抽成 `Worker::serve`，错误在那里变成 `Failed` 或 `ScanSkipped`。
  - 验收用的 autoplay：`ASM_USAGE_AUTOPLAY=<gate 目录>`，脚本 `make screenshot-usage`（用法见 `kb/docs/gpui-kit-guide.md`「脚本化交互与截图」）。
- `kb/docs/`、`AGENTS.md`：见总 plan「各期都要更新的文档」。

### 实施步骤

- [ ] 1. `usage.rs` 的分日函数：先写测试（下节「分日」），再实现；`cargo add --dev chrono-tz -p agent-session-core` 供夏令时的测试用
- [ ] 2. app `model.rs`：改 `RangePreset` 的测试（`day_presets_have_only_a_lower_bound`、`month_presets_cover_whole_calendar_months`、`a_custom_range_includes_its_last_day_and_defaults_to_this_month_so_far` 里的 `midnight` 换成 4:00，加凌晨 4 点前的用例），再改实现
- [ ] 3. `ledger.rs`：先写 `crates/core/tests/ledger.rs` 里只涉及 `Ledger` 的用例（建表、upsert、`refresh_hourly`、版本、损坏改名），再实现
- [ ] 4. `Db` 接上账本：先写经过 `Db` 和 `Indexer` 的用例（写入、只增不删、抢救、核对、两个索引共用），再实现
- [ ] 5. `UsageData` 与 `totals`：先写测试，再实现
- [ ] 6. 在 `tmp/2026-11-12-usage-stats/` 里复制一份索引副本（`sqlite3 index-copy.sqlite ".backup <新文件>"`，不要直接打开备份本身），用它打开一次 `Db`：记录回填耗时、账本大小、第二次打开的核对耗时，与本 spec 的数字对比
- [ ] 7. 核对真实索引有没有在账本上线前被重建过：用 `sqlite3 "file:<路径>?immutable=1"` 只读地比较真实的 `index.sqlite` 和备份 `tmp/2026-11-12-usage-stats/index-copy.sqlite` 里有用量的 session 数（备份是 2,225 个，其中 56 个记录已删）。真实索引少了已删的那批，就在账本第一次在真实数据上运行之后，写一个 `#[ignore]` 的测试 `import_usage_from_an_index_backup`（环境变量给出备份的副本和真实账本的路径），用 `salvage_into_ledger` 把备份抄进账本；做之前先复制一份账本。结果告诉用户并记进 session 总结
- [ ] 8. `store.rs`：先写事件序列的测试，再加 `Usage` 事件和 `quiet` 扫描
- [ ] 9. `main.rs`：状态栏的 Today、自动扫描、`roll_day`；先在 `keyboard_tests.rs` 写用例
- [ ] 10. `make test`、`make parity`；`ASM_DB` / `ASM_STATE` 指到 `tmp/2026-11-12-usage-stats/` 实机验收（下节）
- [ ] 11. 文档、删掉 known-issues 里对应的一条、session 总结，一次 commit

### 测试与验证

core `crates/core/tests/ledger.rs`（新，用 `tests/common` 的 `sandbox()`，账本直接用 `rusqlite` 打开核对）。括号里是对应的验收标准。

- `a_scan_writes_responses_sessions_projects_and_hourly_rows_to_the_ledger`（1）：扫描 fixture 后，账本的调用数等于索引的 `api_usage`；S1 的 session 行有项目 key、标题、类型；`usage_hourly` 按 session 求和等于 `responses` 按 session 求和。
- `a_response_cut_between_two_scans_is_in_the_ledger_once`（1）：沿用 `index.rs::a_response_cut_between_two_scans_is_counted_once` 的做法。
- `deleting_transcripts_then_scanning_and_rebuilding_leaves_the_ledger_unchanged`（2）：记下每个 session 的调用数和 token 合计；删掉 S1 的记录文件和它的 subagent 目录；`scan`，再 `Indexer::rebuild`；账本逐项相等，而索引里 S1 的用量已经没有。
- `a_subagent_deleted_on_its_own_leaves_the_index_but_stays_in_the_ledger`（3）。
- `a_rewritten_transcript_does_not_remove_ledger_rows`（3）：把记录文件改写成更短的内容，索引的调用数下降，账本不变。
- `an_index_of_another_version_is_copied_into_the_ledger_before_it_is_rebuilt`（4）：扫描后用另一个连接删空账本的三张表，`set_user_version_for_test(SCHEMA_VERSION - 1)`，删掉 S1 的记录文件，重新 `Db::open`：账本里有 S1 的调用和它的 session 信息。
- `an_old_index_without_usage_tables_is_rebuilt_without_failing`（4）：手工建一个只有 `sessions` 表、版本号为 1 的库，`Db::open` 成功，账本为空。
- `a_corrupt_index_is_deleted_and_the_ledger_is_left_alone`（5）：扫描后把索引文件写成垃圾，`Db::open` 成功；账本文件的调用数不变。
- `a_corrupt_ledger_is_renamed_aside_and_refilled_from_the_index`（6）：把账本文件写成垃圾，`Db::open` 后目录里有一个 `usage-ledger.corrupt-*.sqlite`，新账本的调用数等于索引；索引没有被删（不扫描就能列出原来的 session，`index_state` 的行数不变）。
- `rows_the_ledger_lacks_are_copied_from_the_index_on_open`（7）：用另一个连接从账本删掉一个 session 的几行并重算它的小时汇总，重新打开后补齐。
- `the_first_open_backfills_sessions_whose_transcripts_are_gone`（8）：扫描、删掉 S1 的记录文件、再扫描（索引把它标成 missing，用量还在），关闭后删掉账本文件，重新打开：账本里有 S1。
- `a_ledger_of_a_newer_version_is_left_alone_and_the_index_still_works`（14）：把账本的 `user_version` 设成 `LEDGER_VERSION + 1`，`Db::open` 和扫描都成功，账本文件的内容和版本号没变，`ledger_usage` 返回 `None`。
- `two_indexes_in_one_directory_share_the_ledger_without_double_counting`（10）：同一个 sandbox 里用 `index.sqlite` 和 `index-cli.sqlite` 各扫一遍。
- `a_failed_scan_leaves_nothing_in_the_ledger`：`Db::transaction` 的闭包返回错误后，`pending` 被清空，账本没有新行。
- `session_info_survives_a_rebuild_that_brings_the_session_back_from_history`：S1 既有记录文件又在 `history.jsonl` 里；删记录、重建后索引里它是 history 行，账本里的 `entrypoint`、`started_at` 仍是原来的。
- `the_ledger_schema_is_what_its_migrations_build`：从版本 0 走完迁移得到的 `sqlite_master` 等于 `ledger.sql` 建出的（现在只有一步，为以后加迁移的人留着）。

core `usage.rs` 的测试（放 `crates/core/tests/usage.rs`）：

- 分日（12）：`the_usage_day_starts_at_four`（+8 下 03:59 属于前一天、04:00 属于当天）；`a_day_starts_at_four_local_in_whole_hour_and_half_hour_zones`（`FixedOffset` +8、−5、+5:30）；`days_around_a_dst_change_are_23_and_25_hours_long`（`chrono_tz::America::New_York`）。
- `totals_cover_the_hours_in_range_and_nothing_else`（11）。
- `costs_follow_the_price_table`（9）：一个桶的费用等于同样 token 用 `pricing::price_tokens` 算出的值；fast 和长 prompt 的桶按各自的倍率；同一份数据用 `totals_with` 传一张把某个模型单价翻倍的价格表，这个模型的费用翻倍，其他模型不变。
- `a_model_without_a_price_costs_nothing_and_is_counted_as_unpriced`（9）。
- `hourly_costs_add_up_to_the_per_response_costs`（9）：对 fixture，`totals` 的费用等于逐行 `pricing::cost` 之和。
- `applying_changed_sessions_replaces_only_those_sessions`。

app：

- `model.rs`：上面步骤 2 的三个用例改写，加 `before_four_today_is_the_calendar_day_before`（12）。
- `store.rs`：`usage_follows_the_first_load_and_later_scans_send_only_changed_sessions`（1、11）：冷启动的事件里 `Loaded` 之后有 `Usage { full: true }`；追加一行后 `Scan`，收到的 `Usage { full: false }` 只含那个 session；`a_rebuild_does_not_drop_usage_the_ui_already_has`（2）：删记录后 `Rebuild`，这次的 `Usage` 不含被删的 session（界面保留原有的）；`a_quiet_scan_that_finds_the_index_busy_is_skipped_without_a_failure`。
- `keyboard_tests.rs`（整个应用在无界面窗口里）：
  - `the_status_bar_shows_todays_cost_and_follows_a_scan`（11）：fixture 的调用时间在过去，先断言 `Today $0.00`；往一个记录文件追加一条时间为现在的 assistant 记录，触发扫描，断言今天的费用等于这条的价格。
  - `ten_minutes_later_an_appended_transcript_shows_up_without_any_input`（13）：追加后 `cx.executor().advance_clock(Duration::from_secs(600))`，等到这一行的 Updated 变化。
  - `scans_triggered_within_thirty_seconds_run_once`（13）：连续调两次 `auto_scan`，只发出一个请求（数 `ScanDone`）。
  - `crossing_four_moves_today_and_the_range_filter`（11、12）：`roll_day` 接受注入的「现在」，换日后 `Filter.since_ms` 变成新一天的 4:00。
  - 窗口激活触发扫描：测试平台能模拟激活就写成用例，不能就直接调激活的处理函数，并在实机验收里覆盖。

实机验收（`ASM_DB`、`ASM_STATE` 指到 `tmp/2026-11-12-usage-stats/accept/`，`ASM_CLAUDE_DIR` 指到复制出来的一小份数据目录；不动真实索引和真实账本）：

- 启动，状态栏有 `Today $X`；用 `sqlite3` 对账本按今天 4:00 以后求和，结合价格表核对 X。
- 删掉数据目录里一半的记录文件，Scan for Changes，再 Rebuild Index；截图前后的状态栏，Today 不变；`sqlite3` 核对账本的调用数不变。
- 用真实索引的副本（步骤 6）量回填和核对的耗时，写进本 spec 的「实测」。
- 开着应用 10 分钟以上，期间在另一个终端跑一次 Claude Code，确认表格和 Today 自己更新。

### 风险与待定

- **账本上线前索引很可能先被重建。** 规划时工作区里还有把 `SCHEMA_VERSION` 从 5 提升到 6 的改动（`asm` CLI 改了 prompt 的判定）；core 实现时已作为 `bd56682` 提交。它合并后，任何用新构建打开真实 `index.sqlite` 的动作（`make dev`、截图脚本、发版后的应用）都会重建索引，已删 session 的用量随之丢失；`asm` 自己用的是 `index-cli.sqlite`，不受影响。备份 `tmp/2026-11-12-usage-stats/index-copy.sqlite` 里有这批用量，实施步骤 7 负责把它找回来。账本这一期合并之后就不再有这个问题：重建前会先抢救。
- **自动扫描后表格会滚回选中的行。** `after_rows_changed` → `select_row` 在选中的 session 换了位置时调 `set_selected_row`，它会顺带滚动（gpui-kit 没有只选中不滚动的入口）。手动扫描时这不是问题，每 10 分钟自动来一次就可能在用户往下翻的时候把表格拉回去。实现时先实测；确实打扰的话，自动扫描引起的刷新里，选中行不在当前可见范围内时不调 `set_selected_row`，只更新 `selected`。
  - **实测与处理（2026-11-16）**：无界面测试能复现：窗口压到 260px 高、选中第一行、滚到底，往排在最后的记录文件追加一条后自动扫描，滚动位置从 −86px 回到 0。没有照上面的做法（只更新 `selected`、不调 `set_selected_row`）：那样表格的选中行号还指着旧位置，高亮会落在另一个 session 上。改为照常 `set_selected_row`，但在同一个 update 里把它放进 `vertical_scroll_handle` 的延迟滚动请求拿掉（`deferred_scroll_to_item` 是公开字段，下一帧 prepaint 才执行）。范围是所有扫描引起的刷新（含菜单的 Scan for Changes），只对「同一个 session 换了行」生效；新选中的行、过滤和排序照常滚动。用例 `keyboard_tests.rs::an_automatic_scan_does_not_scroll_the_table_back_to_the_selected_row`。真实窗口里的手感留给实机验收。
- **两个索引的解析版本不同。** `index-cli.sqlite` 的 schema 比已发布的 app 新，两边对同一次调用解析出的值如果不同，账本里是后写的那个。目前用量字段两边相同。
- **账本和会话表的数字可以不同**：只增不删带来的（subagent 被单独删、文件被重写）。这是定下的语义，不登记为问题。
- **核对只比调用数。** 账本里某个 session 多出几行（subagent 被单独删过）、同时又缺了另外几行时，两边的数可能相等或账本更大，这次缺的就补不上。要同时发生「只增不删造成的多」和「崩溃造成的少」，而后者在先提交账本的顺序下本来就不该出现，所以不为它付逐行比对的代价（110 ms，且随历史变慢）。
- **`is_autocommit` 判断是否在事务里。** 已由 `unchecked_transactions_disable_autocommit` 确认：`Transaction::new_unchecked` 开启后为 false，提交后恢复 true。
- **回填时读 `api_usage` 的内存**：12.9 万行约十几 MB，一次性。一年后的索引如果还留着上百万行，按 session 分批读写。
- **实测数字的来源**（2026-11-16，release，备份 129,372 次调用、2,225 个有用量的 session）：原 SQL 估计为调用表 14 MB、小时汇总 5,510 行 / 0.44 MB，Rust 回填预计约 1 秒。实际 Rust 回填 **356.324 ms**，关闭连接后的完整账本 **17,485,824 字节（16.68 MiB）**，第二次 `Db::open`（包含正常 schema 检查和双方调用数核对）**2.145 ms**；小时汇总仍为 **5,510 行**。回填约为估计的 36%，核对落在预估 2–5 ms 内；完整文件包含 session、projects、索引与页空隙，不能直接与只量调用表的 14 MB 比较。
  - 备份本身实际是 **SCHEMA_VERSION 4**，不是规划时的 5，也不是当前代码的 6。先按 brief 用 `immutable=1` 的 SQLite backup 复制到任务目录；为单独测回填/核对，再在当前 schema 的空索引中复制备份的 projects、sessions、api_usage、session_model_usage 四张表（数据原样，消息和文件扫描状态不参与此测量）。准备脚本为 `tmp/2026-11-12-usage-stats/usage-p1-core/prepare-bench.py`，结果为同目录 `bench-final.log`。
  - 另用版本 4 的原样副本测升级路径：第一次打开 **1,347.400 ms**（包含抢救与旧索引删除重建），账本 **17,637,376 字节**，保留 **3,361 个 transcript session 的元数据**（含无用量者）、**129,372 次调用**、**5,510 个桶**。第二次打开 1.787 ms 的索引已为空，不能把这个数当成有真实用量时的核对耗时；结果见 `bench-old-index.log`。
  - 原 SQL 试验的按两张大表比调用数 38 ms、逐主键比较 110 ms 仍作为方案对比；本实现核对小的汇总表。原试验未发现 `ts=0` 或无价格调用。真实数据目录与原始备份从未被开发构建打开。
