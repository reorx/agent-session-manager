---
created: 2026-10-26
tags:
  - favorites
  - persistence
---

# Session 收藏（Favorites）实现方案

## PRD

### 背景

Claude Code 按 `cleanupPeriodDays`（本机 60 天）删除 `~/.claude/projects/<project>/<id>.jsonl` 和同名目录。删除后索引只剩一行 History，正文、搜索和费用明细都没有了。这次要加「收藏」：把一个 session 的文件快照复制到独立的收藏目录。扫描时同时列出两处，按 session id 去重，**收藏的副本始终优先**；原文件被清理后，这个 session 照常显示和搜索。

已和用户确认：
- 默认收藏目录放在索引所在目录：`~/Library/Application Support/dev.reorx.AgentSessionManager/favorites/`，可以在菜单里换。
- 换目录时把旧目录的收藏**迁移**到新目录；遇到同 id，保留新目录里那一份。
- 取消收藏会把文件**移到废纸篓**。如果原 session 已被清理（收藏是最后一份），先弹原生确认框。
- 入口：菜单栏 Favorites 菜单、⌘D、表格行右键菜单、详情面板 ★ 按钮，外加「仅看收藏」过滤。

## Tech Design

### 设计要点

### 收藏目录布局
`<fav>/claude/projects/<project_key>/<id>.jsonl`，加上整个 `<id>/` 目录（`subagents/*.jsonl`、`*.meta.json`、`tool-results/`）。`<fav>/claude` 本身就是一个合法的 `ClaudeDir`，所以 `discover_transcripts` / `discover_subagent_transcripts` 不用改就能复用，`asm --claude-dir <fav>/claude list` 也能直接读。路径里留一层 `claude/`，是给二期的其他 agent 预留位置。

### 收藏目录从哪里来（core 与 `asm` 共用）
新建 `crates/core/src/config.rs`，按以下顺序决定：
1. `$ASM_FAVORITES_DIR`
2. 索引同目录 `config.json` 里的 `favorites_dir`
3. 默认：索引所在目录下的 `favorites/`

`asm` 和 app 读的是同一份配置，CLI 扫描同一个索引时不会出现两边不一致。把 `ASM_DB` 指到 `tmp/` 后，收藏目录也跟着落到 `tmp/`，因此验收和 `make parity` 都碰不到真实收藏。`state.json` 仍只存视图状态，不放这个设置。

### 复制（`crates/core/src/favorites.rs`）
- 在 `<fav>/claude/.staging/<id>/` 拼好完整的一份：`<id>.jsonl` 和 `<id>/` 目录。staging 必须在 `projects/` 之外，因为 `discover_subagent_transcripts` 会把 `projects/<key>/` 下的任何子目录都当成 session 目录。staging 和收藏在同一卷上，所以后面的 rename 是原子的。
- 递归复制的每个文件都用 `File::set_modified` 保留 mtime，Updated 列不变。`std::fs::copy` 在 APFS 上走 clonefile，几乎不花时间和空间（推测，实现时实测）。
- `*.jsonl` 复制后截断到最后一个 `\n`：正在写的 session 可能带着半行，不截掉的话这半行会被永久冻结在快照里。截断 clone 出来的文件不影响原文件。
- 落位顺序：先 rename `<id>/`，**最后** rename `<id>.jsonl`。discover 以主 `.jsonl` 为准，所以扫描永远看不到半个收藏。
- 目标已存在时报错 `already a favorite`。这次不做刷新，再按一次 ⌘D 就是取消收藏。session id 里不能含 `/` 或 `..`。

### 取消收藏
`favorites::remove` 先把 `<id>.jsonl` 移走，再移走 `<id>/`。去处由 `Trash` 枚举决定：
- `Trash::System`：macOS 废纸篓，新增 `trash` crate，指定 NSFileManager 方式，不走 Finder AppleScript。
- `Trash::Dir(path)`：rename 进给定目录，供测试和验收用。

app 默认用 `System`；设了 `$ASM_TRASH_DIR` 时用 `Dir`。**测试一律用 `Dir(tempdir)`，不碰真实废纸篓。**
`favorites::original_exists(claude, project_key, id)` 判断原文件还在不在，UI 用它决定要不要弹确认框。

### 换目录与迁移
`favorites::migrate(old, new)`：
- 逐个 session 移动 `.jsonl` 和目录。新目录已有同 id 的，跳过并保留旧文件。
- 能 rename 就 rename，跨卷时复制后再删旧文件。
- 返回 `{moved, skipped}`。

两边都先 canonicalize，再拒绝以下几种目标：和旧目录相同；在旧目录之内或包含旧目录；在 Claude 目录（`~/.claude`）之内。全部移完才写 `config.json` 并更新索引 meta。中途失败时，把已移动的文件挪回旧目录，并报告失败原因。

### 发现与去重（`crates/core/src/claude/discover.rs`）
- `TranscriptFile` 加 `favorite: bool`。
- 新增 `discover_all(claude, favorites: Option<&ClaudeDir>) -> Discovered { sessions, subagents }`，规则只在这一处：
  - 收藏的 session id 集合**只认主 `.jsonl`**；
  - 收藏的 subagent 只保留其 session id 在这个集合里的；
  - 原目录里 session id 属于这个集合的主文件和 subagent 一律丢掉。
- 所有 discover 调用方都改用它：索引器、`regex.rs::list_transcripts`（`asm search` 非索引模式）、`transcript.rs::resolve_transcript`（`asm transcript`、`transcript-reader`）。
- **错误不再静默跳过**：只有 NotFound 可以跳过，比如收藏根目录不存在、session 没有 `subagents/`、读目录和 stat 之间文件被删。其余 IO 错误（EPERM 等）让整次扫描失败。原因：在新的「离开」规则下，漏掉一个文件就等于把它当成已删除，会清掉它的 messages。

### 索引器的状态转换（`crates/core/src/index/indexer.rs`）
要守住的不变量：**扫描结束后，`index_state` 里只能有本次去重后 discover 到的路径**。每个 (session, agent) 最多只有一条路径带状态，所以每次「胜出的那份」换了，新路径都没有状态，必然 Full 解析。

- `Indexer::new(claude, favorites: Option<ClaudeDir>)`。
- `Gone::find` 改为「状态路径不在本次 discover 出的路径集合里」，不再看 `path.is_file()`，并且在**解析之前**就算出来。设 `live_ids` = 本次 discover 到的 session 主文件的 id：
  - 离开的 session 状态，其 id 在 `live_ids` 中：表示被另一份取代，只 `delete_index_state`。**不能**再 `delete_messages` 或 `delete_usage(sid, "")`，因为那会抹掉同一事务里胜出那份刚写进去的行。保险起见，被取代的 sid 把胜出路径的 plan 强制设成 Full。
  - 离开的 session 状态，其 id 不在 `live_ids` 中：沿用 `mark_missing`，`mark_transcript_missing` 同时把 `favorite` 置 0。
  - 离开的 subagent 状态：所属 session 在 `live_ids` 中，就 `delete_usage(session, agent)` 并把 sid 加进 `touched`；不在则保留 usage（整个 session 被清理的情形）。然后 `delete_index_state`。仍在 `write_subagent` 之前执行，两者共用主键。
  - `ScanReport` 加 `files_superseded`。
- **根目录守卫**：新增 `meta(key TEXT PRIMARY KEY, value TEXT)` 表，记录建索引时用的 `claude_root` 和 `favorites_root`。扫描时解析出的根目录和记录的不一致，就返回 `RootsChanged` 错误，Rebuild 除外。
  - 原因：去掉 `is_file()` 之后，`asm --claude-dir X` 或换了 `ASM_FAVORITES_DIR` 的进程扫同一个索引，会把对方的 session 全部当成离开，造成破坏。现在这种扫描是无害的。
  - CLI 遇到 `RootsChanged` 时提示用 `--db` 或 `index --rebuild`。app 遇到时（只可能是环境变量变了）自动 Rebuild。换收藏目录时由 store 线程在迁移后更新 meta。`clear_all` 同时清空 meta。
- Schema 改动：
  - `sessions.favorite INTEGER NOT NULL DEFAULT 0`，同步改 `SessionSummary.favorite`（在 `empty()` 里给默认值）；
  - `SESSION_COLUMNS` 把它加在**末尾**（第 40 列），不挪动前面的下标，`row_to_session` 和 `upsert_session` 的占位符一并补上；
  - 新增 `meta` 表；
  - `SCHEMA_VERSION` 从 2 升到 3，整库重建。
- `write_parsed` 和设 `file_path` 一样，从 `TranscriptFile.favorite` 设 `favorite`。`ListFilter` 加 `favorites_only`。

### store 线程（`crates/app/src/store.rs`）
- `Worker` 持有 `FavoritesDir`（root + trash）。新增三个请求：
  - `StoreRequest::Favorite(id)`：从 `db.session(id)` 取 `file_path` / `project_key`，执行 `favorites::save`，然后跑增量 scan。
  - `StoreRequest::Unfavorite(id)`：`favorites::remove`，然后 scan。
  - `StoreRequest::SetFavoritesDir(Option<PathBuf>)`：`None` 表示恢复默认。依次执行 migrate、写 config、更新索引 meta 的 `favorites_root`、换掉 indexer 里的收藏根目录、scan。
  - 启动扫描时遇到 `RootsChanged`，自动走 Rebuild，并在状态栏说明原因。
- 新增事件 `StoreEvent::FavoriteChanged { id, favorite }` 和 `StoreEvent::FavoritesDirChanged { dir, moved, skipped }`，UI 收到后用 `push_notification` 提示。行本身的更新走现有的 `Session` / `Loaded` 事件。
- 文件操作都在这条线程里，和扫描串行执行。

### 界面（`crates/app/src`）
- `AppPaths` 加 `favorites`，由 `config` 解析；测试里放 tempdir，trash 也用 `Dir(tempdir)`。
- `SessionRow::from_summary` 加 `favorite: bool`。`model.rs` 的 `Filter` 加 `favorites_only`，并持久化到 `ViewState`（`#[serde(default)]`）。
- **动作** `ToggleFavorite`（⌘D）由 `AppRoot` 处理：
  - History 行没有 transcript，提示「无法收藏」。
  - 已收藏且 `original_exists == false`：弹 `window.prompt(PromptLevel::Warning, …, ["移到废纸篓", "取消"])`，确认后才发 `Unfavorite`。
  - 其余情况直接发 `Favorite` / `Unfavorite`。
- **菜单栏** 新增 Favorites 菜单，含四项：Toggle Favorite ⌘D、Reveal Favorites Folder（`cx.reveal_path`）、Change Favorites Folder…（`cx.prompt_for_paths(PathPromptOptions { directories: true, files: false, multiple: false, .. })`）、Use Default Favorites Folder。设了 `$ASM_FAVORITES_DIR` 时，后两项点了只提示「由环境变量指定」。
- **右键菜单**：`SessionTableDelegate` 实现 `TableDelegate::context_menu(row_ix, ..)`，提供「收藏 / 取消收藏」和「在 Finder 中显示」，后者 reveal transcript 路径。
- **★ 标记**：标题单元格在文字前单独放一个 ★ 元素，不拼进 `row.title`，避免打乱搜索高亮区间。颜色写进 `style.rs`。
- **详情面板**：标题行（`detail.rs` 约 226–248 行）加 ★ 按钮，切换收藏；Metadata 里加一项 Favorite，显示收藏路径或「—」。
- **过滤**：Kind 下拉末尾加分隔线和「★ Favorites only」开关，和现有 Kind 选择叠加使用；按钮值上体现过滤状态。
- **焦点**：确认框、文件夹选择器关闭后把焦点还给表格（AGENTS.md 的焦点规则）。

### CLI（`crates/core/src/bin/asm.rs`）
- `asm favorite <id|prefix>`、`asm unfavorite <id|prefix> [--yes]`：原文件已不在时，没带 `--yes` 就拒绝执行。
- `asm favorites-dir [<path>|--reset]`：查看或迁移收藏目录。
- `asm list --favorites`。
- `asm session` 的输出显示 `favorite: <path>`。

### 实施步骤（BDD：先写测试）

1. **core 测试先行**，写在 `crates/core/tests/favorites.rs`。`tests/common/mod.rs` 的 `Sandbox` 加 `fav_root()`，再加一个用 core `favorites::save` 的 `favorite(sb, project, sid)` helper；trash 一律用 `Trash::Dir(tempdir)`。用例：
   1. `favoriting_moves_the_row_to_the_copy_without_changing_it`：`file_path` 在收藏下，`favorite` 为真；费用为 `S1_MAIN+SUBAGENT`，10 条消息，`updated_at == MTIME_S1`；只有收藏路径带 index_state；`files_missing == 0`；这一行只流出一次。
   2. `a_favorite_outlives_its_deleted_original`：原 jsonl 和目录删掉后重扫没有任何变化，行没有变成 missing。
   3. `the_favorite_wins_while_the_original_keeps_growing`：原文件追加后重扫没有变化，原文件没有 index_state。
   4. `unfavoriting_falls_back_to_the_grown_original`：`last_prompt` 是追加的那句，`favorite` 为假，费用不重复。
   5. `unfavoriting_after_the_original_is_gone_leaves_a_history_only_row`：行变成 missing，`favorite` 为假，没有 messages，费用保留。
   6. `a_fresh_index_equals_the_incremental_one`：比较 `session`、`messages`、`usage`，即「全量 == 多次增量」。
   7. `a_favorites_subagent_counts_once`：`subagent_count == 1`。
   8. `a_missing_favorites_root_is_not_an_error`。
   9. `an_unreadable_favorites_root_fails_the_scan_and_marks_nothing`：用 chmod 000 制造。
   10. `a_half_copied_favorite_is_ignored`：只有 subagents 或 staging 时，仍用原文件，usage 不变。
   11. `migrating_the_root_reindexes_from_the_new_place`。
   12. `migration_keeps_the_new_roots_copy_on_conflict`。
   13. `migration_rejects_nested_or_claude_roots`。
   14. `rebuild_keeps_favorites_whose_original_is_gone`。
   15. `favoriting_a_growing_file_stops_at_the_last_newline_and_keeps_the_mtime`。
   16. `favoriting_twice_is_an_error`。
   17. `a_scan_with_other_roots_fails_with_roots_changed`。
   18. `resolve_transcript_and_list_transcripts_see_favorites`。
2. 实现 `config.rs`、`favorites.rs`、`discover_all`、schema 与 `db.rs`、`model.rs`、`indexer.rs`、`transcript.rs`、`regex.rs`，直到第 1 步全部通过。`trash` crate 的 macOS 删除方式（NSFileManager）以本地源码为准，加依赖后再查。
3. CLI 子命令，并在 `tests/cli.rs` 里补用例：收藏和取消收藏的往返、`--yes` 的保护、`list --favorites`、`favorites-dir` 迁移、根目录不一致时的报错文本。
4. `store.rs` 的请求与事件，`store.rs` 自带的测试加 Favorite / Unfavorite 往返。
5. UI：`keyboard_tests.rs` 先写行为测试：
   - ⌘D 收藏，行出现 ★，tempdir 里出现文件；
   - 再按 ⌘D 取消收藏，原文件还在，不弹确认框；
   - 原文件不在时有待处理的确认框（`has_pending_prompt`），`simulate_prompt_answer` 选「取消」后文件仍在；
   - Change Favorites Folder 用 `simulate_path_prompt_response`，文件被迁移，焦点回到表格；
   - 「仅看收藏」过滤只显示收藏的行。

   然后实现动作、菜单、右键菜单、★ 和详情按钮。
6. 文档：
   - 把方案存为 `kb/plans/2026-10-26-favorites.md`（/kb）；
   - 更新 `kb/docs/claude-session-data-format.md`（收藏目录与优先规则）、`kb/docs/app-architecture.md`（新请求与事件）、`kb/docs/product-prd.md` 状态表、AGENTS.md（当前状态、环境变量 `ASM_FAVORITES_DIR` / `ASM_TRASH_DIR`、`asm` 子命令），并从 `kb/todo.md` 删掉 "save session to consistent place"；
   - 最后 commit。

### 验证

- `cargo test -p agent-session-core`、`cargo test -p agent-session-manager`、`make test`。
- `make parity`：`ASM_DB` 在 `$OUT`，收藏目录默认落在 `$OUT/favorites`（空），结果应和改动前一样。改了 discover 的错误处理，这一步必须跑。
- 真实索引会因为 `SCHEMA_VERSION` 升级而整库重建一次，约 4 秒，属于预期。
- 真实数据验收，`ASM_DB=tmp/<date>-favorites/index.sqlite`、`ASM_TRASH_DIR=tmp/<date>-favorites/trash`：
  1. `make dev`，⌘D 收藏一条真实 session，核对 `tmp/.../favorites/claude/projects/...` 下的文件和 mtime，表格出现 ★。
  2. 右键菜单和详情面板 ★ 按钮各点一次（cliclick 只做鼠标点击），再打开「仅看收藏」过滤。
  3. 「原文件被清理」用一个临时 Claude 目录模拟：`ASM_CLAUDE_DIR=tmp/<date>-favorites/claude`，从真实数据里复制两个 session 过去，收藏其中一个后删掉它的原文件（字面路径，不用变量 rm），然后重扫，确认这一行仍是 CLI 类型、带 ★、能搜到。
  4. 截图后用 Read 查看，归档到 `tmp/2026-10-26-favorites/`；验收结束 kill app（`pgrep -fl agent-session-manager`）。
- `asm favorite / unfavorite / list --favorites / favorites-dir` 在 tmp 索引上各跑一遍。

### 后续可做（不在本次范围）

- 「刷新收藏」：原 session 还在增长时重新复制一次快照。
- 「恢复到 ~/.claude」：原文件被清理后 `claude --resume <id>` 会失败，把收藏复制回去就能 resume（推测可行，需验证 Claude 是否接受）。
- 把收藏目录放进 iCloud Drive 时，未下载的占位文件（dataless）在 mmap 时的表现需要实测（推测：读取会触发下载，可能拖慢扫描）。
