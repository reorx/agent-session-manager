---
created: 2026-10-26
tags:
  - m4
  - m4b
  - detail-panel
  - actions
  - export
  - spec
  - agent-session-manager
---

# M4b：详情面板的操作入口、History 对话、代码块与按回合跳转

总 plan：[M4 详情面板](2026-10-26-m4-detail-panel.md)。依赖 [M4a](2026-10-26-m4a-turn-cards-spec.md) 的读取线程、回合卡和行表，M4a 合并后才开始。对应画稿 2（代码块）、5、6，以及画稿 1–4 标题行的操作按钮。

## PRD

### 背景

M4a 之后能在详情里看完整对话，但看完要做的事还没有入口：接着这个 session 干活要复制 resume 命令，交给别人要 session ID 或导出的 TXT，查问题要找到记录文件和项目目录，这些现在只能在 Metadata 里手动选文字。另外，记录文件已被清理的 History session 在 Conversation 里只有一句说明；回复里的代码块和正文混在一起；长 session 里只能靠滚动找回合。

### 目标

在详情标题行、表格右键菜单和快捷键三处都能一步完成复制、定位和导出；History session 也能看到它的 prompt；回复里的代码一眼可辨；用键盘在回合之间跳。

### 功能与交互

#### 1. 标题行的操作按钮

画稿 1 的标题行右侧，从左到右：

| 按钮 | 样式 | 动作 |
|---|---|---|
| ★ / ☆ | 图标按钮（就是现在的收藏按钮，挪进这一组） | 收藏 / 取消收藏，行为不变 |
| Copy resume | 带图标的文字按钮 | 复制恢复命令 |
| 复制 ID | 图标按钮 | 复制完整的 session ID |
| 在 Finder 中显示 | 图标按钮 | 在 Finder 里选中这个 session 的记录文件（收藏了的就是收藏副本） |
| 打开项目目录 | 图标按钮 | 在 Finder 里打开项目目录 |
| 导出 TXT | 图标按钮 | 见第 4 节 |

- 每个按钮都有悬停提示，写出它做什么；置灰时提示写出原因。
- **恢复命令**是 `cd <项目目录> && claude --resume <session id>`，粘到终端直接能用。项目目录不存在时的写法见"风险与待定"第 1 条，以验证结果为准。
- 复制成功后弹一条简短的通知（"已复制 resume 命令"、"已复制 session ID"），和现有的收藏通知同一种样式。

什么时候置灰：

| 情况 | 置灰的按钮 | 悬停提示 |
|---|---|---|
| History session（记录文件已被 Claude Code 清理） | ★、Copy resume、在 Finder 中显示、导出 | "The transcript is gone" |
| 没有 session ID 的早期 History session | 以上再加复制 ID | "This session has no ID" |
| 收藏过、但原始记录文件已被清理 | Copy resume | "Only the favorite copy is left; Claude Code can no longer resume it" |
| 项目目录已不存在 | 打开项目目录 | "The project folder no longer exists" |

#### 2. 表格行的右键菜单

画稿 6，在现有的两项上扩展，全部改成英文：

```
Favorite / Unfavorite            ⌘D
─────────────
Copy Session ID                  ⌘C
Copy Resume Command             ⇧⌘C
  cd ~/Code/agent-session-manager && claude --resume 3c1e9a20-…
─────────────
Reveal Transcript in Finder
Open Project Folder
Export Transcript as TXT…
─────────────
Hide Project "agent-session-manager"
```

- 菜单作用于右键点中的那一行，不一定是当前选中的行。
- "Copy Resume Command"下面用小一号的灰色等宽字预览这条命令。
- 置灰规则和标题行按钮相同。
- "Hide Project"和 View 菜单里的"Hide Project of Selected Session"效果一样，但对象是右键点中的那一行。

#### 3. 快捷键

- 表格有焦点时：`⌘C` 复制选中 session 的 ID，`⇧⌘C` 复制它的恢复命令。搜索框里的 `⌘C` 仍是复制文字。
- 不能复制时（见上表），弹一条通知说明原因，不复制任何东西。

#### 4. 导出 TXT

- 从标题行按钮或右键菜单进入，弹出系统的保存对话框。默认文件名 `<项目名>-<session ID 前 8 位>.txt`；默认目录是这次运行里上一次导出的目录，第一次是"下载"文件夹。
- 导出的内容和现有的 `transcript-reader` / `asm transcript` 完全一样：用户输入、最终回复和每个回合的工具调用次数。
- 写完弹通知"已导出 <文件名>"。记录文件里有解析不了的行时，通知里加一句"有 N 行无法解析，已跳过"。
- 顺带修正导出的一处错误：被用户打断时留下的"[Request interrupted by user]"现在会被当成一条用户输入导出，改为不导出。

#### 5. History session 的 prompt 列表

画稿 5。记录文件已被清理、只剩 `history.jsonl` 里 prompt 的 session：

- 顶栏写"3 prompts from history.jsonl"。
- 顶栏下面一行灰字："Claude Code cleaned up this session's transcript. Only its prompts in history.jsonl are left: no replies, tool calls or cost."
- 每个 prompt 一张卡，标题 `TURN n` 和时间，内容是 prompt（粘贴的长文本已还原），没有状态灯、刻度条和回复。prompt 同样折叠到 4 行。
- 搜索命中照常高亮，可以用 `n` / `⌘G` 等在 prompt 之间跳。

#### 6. Metadata 里的"修改过的文件"

Metadata 的 Top tools 下面加一项 Files：写"12 files"，下面列出 session 里改过的文件，项目目录下的写相对路径。超过 10 个时只列前 10 个，后面是"Show all · 12 files"。没有改过文件的 session 写"—"。

#### 7. `[` / `]` 按回合跳转

- 表格有焦点、详情显示 Conversation 时，`]` 跳到下一张卡（回合或 AUTO）的开头，`[` 跳到上一张。已经在第一张或最后一张时不动。
- 跳转不改变卡片的展开状态。
- 没有搜索时，顶栏中间显示提示"[ ] jump between turns"（键帽样式）；有搜索时这个位置让给"▲ 3 / 9 hits ▼"。

#### 8. 代码块与行内代码

画稿 2。prompt、过程文字、最终回复里：

- 用 ``` 包起来的代码块显示成浅灰底、细边框的等宽框，不换行（超宽的部分截掉，展开后也是这样），不做语法高亮。没有闭合的 ``` 一直算到这段文字的末尾。
- 单个反引号包起来的行内代码用等宽字和浅灰底显示。
- 折叠仍按原文行数算，代码块的行也算在内。代码块里的搜索命中同样高亮。
- 这三处以外（工具的 input / output、thinking）不做分段，本来就是等宽或纯文本。

### 不做的事（本期）

- 完整 Markdown（标题、列表、表格、加粗）、语法高亮
- 在应用里直接拉起终端执行 resume（M5）
- 把收藏副本恢复到 `~/.claude` 以便 resume（收藏方案的后续工作）
- 查看 subagent 的对话（M5）
- 菜单栏不加新菜单项

### 验收标准

1. 标题行有 ★、Copy resume 和四个图标按钮，顺序和画稿一致，悬停有提示
2. Copy resume 复制的命令粘贴到终端能直接恢复这个 session
3. 复制 ID、在 Finder 中显示、打开项目目录都做到它们说的事；收藏的 session 在 Finder 里选中的是收藏副本
4. History session、无 ID 的 session、原文件被清理的收藏、项目目录不存在，四种情况按表置灰，提示写出原因
5. 右键菜单按画稿 6 的顺序列出各项，作用于右键点中的那一行
6. 表格有焦点时 `⌘C` / `⇧⌘C` 复制 ID / 恢复命令；搜索框里的 `⌘C` 不受影响
7. 导出的文件和 `transcript-reader` 导出同一个 session 的结果逐字一致；打断标记不再出现在 `<user>` 里
8. History session 的 Conversation 是 prompt 卡片，粘贴文本已还原，能搜索和跳转
9. Metadata 列出修改过的文件
10. `[` / `]` 在卡片之间跳，到头不动；顶栏的提示和命中导航按有无搜索切换
11. 回复里的代码块是等宽框，行内代码是等宽浅灰底，里面的命中照样高亮

## Tech Design

### 现状

以 M4a 完成后的代码为准，开工前先读 M4a 的 session 总结，确认下面这些名字没有变。

- **标题行**：`detail.rs::render_detail_header`，★ 是 `Button::new("favorite-session")`，点击走 `AppRoot::toggle_favorite`（`crates/app/src/favorites.rs`）。History session 收藏时会提示"无法收藏：没有 transcript"。
- **右键菜单**：`table.rs::context_menu`（`SessionTableDelegate` 实现的 `TableDelegate` 方法）现在有两项：`menu("收藏"/"取消收藏", FavoriteSession { id })` 和"在 Finder 中显示"（直接 `cx.reveal_path`）。带 id 的 action 写法见 `favorites.rs` 的 `FavoriteSession`（`#[derive(gpui_kit::Action)] #[action(namespace = app, no_json)]`）。
- **gpui-component 的菜单**：`PopupMenu::menu(label, action)`、`menu_with_disabled`、`separator`、`item(PopupMenuItem)`；`PopupMenuItem::element(builder)` 可以自定义整项内容，`.disabled(bool)`。菜单项右侧的快捷键由 `render_key_binding` 按 action 相等去查绑定（`gpui-component-0.6.1/src/menu/popup_menu.rs:1094`）。
- **快捷键**：`main.rs::bind_keys`；`cmd-d` 是收藏，`n` / `shift-n` 绑在 `"DataTable"` 上下文。`cmd-c`、`cmd-shift-c`、`[`、`]` 目前没有占用，DataTable 自身的绑定里也没有（`data_table.rs:18–28`）。
- **平台接口**（`gpui-pre-0.3.5/src/app.rs`）：`write_to_clipboard(ClipboardItem)`、`prompt_for_new_path(directory, suggested_name)`、`reveal_path`、`open_with_system`。
- **图标**：`main.rs` 用 `with_assets(gpui_kit::assets::Assets)`，只带 101 个默认图标；其中有 `copy`、`folder-open`、`star`、`star-fill`，没有画稿用的 `square-terminal`、`file-search`、`download`（`gpui-kit-assets-0.6.1/assets/icons/` 里都有）。gpui-kit-assets 的 README "Default component icons and application extras" 一节写了怎么用自己的 `AssetSource` 补几个图标、其余回落到 `Assets`。
- **恢复命令**：`SessionRow.resume` 现在是 `claude --resume {id}`（`model.rs::from_summary`），Metadata 的 Resume 一行显示它；测试 `a_row_shows_the_project_folder_name_and_a_short_id` 断言了这个字符串。
- **收藏**：`favorites::original_exists(claude, project_key, id)` 判断原始记录文件是否还在。`SessionRow.transcript_path` 对收藏的 session 指向收藏副本。
- **导出**：`transcript::read_transcript(path) -> TranscriptExport { text, skipped_lines }`；`human_text` 在 M4a 拆成了 `is_human_input` + 取文本。打断标记能通过 `is_human_input`（M4a 的 conversation 在调用它之前先判断打断，导出没有）。
- **History**：`claude/history.rs::history_sessions(project_key, rows, cache_dir)` 把一个项目的 history 行分组成 session：有 `sessionId` 的按 id 分；没有的（2025-11 之前）按 2 小时间隔切，id 是 `history:<project_key>:<首条时间戳>`。`expand_pasted` 还原粘贴文本，`load_history(path)` 读整个 `history.jsonl`。
- **修改过的文件**：`SessionSummary.files_touched`（来自 `file-history-delta` 记录的 `trackingPath`），`Db::list_sessions` 经 `attach_details` 填好，`SessionRow` 还没有对应字段。
- **M4a 的行表**：`Row::{Head, Step, Tail, Compact}`，`conversation.rs::build` / `rows`，`fold`。

### 方案

所有操作收进一个新的 app 模块 `actions.rs`：先由纯函数根据一行的事实（有没有 id、有没有记录文件、原文件在不在、项目目录在不在）算出每个操作是否可用及原因，再由 `AppRoot` 的 action 处理函数执行。标题行按钮、右键菜单、快捷键三个入口都派发同一组 action，所以行为只写一遍。导出和读 History 都放在 M4a 的读取线程上。代码块分段是 app `conversation.rs` 里的纯函数，渲染时按段画。

关键决策：

1. **action 带 `id: Option<String>`**：右键菜单派发 `CopyResume { id: Some(..) }`，作用于点中的行；快捷键绑定 `CopyResume { id: None }`，作用于选中的行。否决：右键时先选中那一行（会触发详情重读，且和 macOS 的习惯不符）。代价：菜单按 action 相等查快捷键，`Some(id)` 查不到 `None` 的绑定，⌘C / ⇧⌘C 的提示要用 `PopupMenuItem::element` 自己画在右侧。
2. **可用性检查要碰文件系统，只在需要时做**：选中一行时（给标题行用）、打开右键菜单时、执行 action 时各做一次 `is_dir` / `is_file`，每次两三个 stat。不放进 `SessionRow::from_summary`：那是整表加载的路径，几千行逐个 stat 不值得。`SessionTableDelegate` 需要 `ClaudeDir` 才能判断原文件，创建时传进去。
3. **`SessionRow.resume` 删掉**，换成 `actions::resume_command(row, dir_exists) -> Option<String>`；Metadata 的 Resume 一行显示选中时算好的命令。
4. **读取线程的队列里只取代对话请求**：`ReadRequest` 变成枚举 `Conversation { .. } | History { .. } | Export { .. }`；M4a 的"只留最后一个"逻辑只丢弃旧的 Conversation / History，Export 一律执行。
5. **History prompt 列表复用分组逻辑**：把 `history_sessions` 里分组的部分抽成 `group_history(project_key, rows) -> Vec<HistoryGroup>`，新函数 `history_prompts(claude, project_key, session_id) -> Result<Vec<HistoryPrompt>>` 和 `history_sessions` 都用它，保证 prompt 列表和表格里的 session 一一对应。结果转成 M4a 的 `Conversation`（每个 prompt 一个只有 prompt 的 `Turn`，`end = NoReply`），命中标记照旧；app 侧用一个标志让卡片不画状态灯和刻度，不显示"no reply"。
6. **代码块分段在 app 做**：`segments(text) -> Vec<Segment>`，`Segment::Text { range, inline_code: Vec<Range> }` 或 `Segment::Code { range }`，区间都指向原文，命中区间不用换算，渲染时按段切开。行内代码和命中高亮合成一组 `TextRun`（`StyledText::with_runs`）：行内代码换等宽字体和浅灰底，命中换黄 / 橙底，两者重叠时命中的底色优先。
7. **导出走读取线程**：`Store::export(path, dest)`，线程里调 `read_transcript` 再写文件，回 `StoreEvent::Exported { dest, skipped_lines }`，失败走 `StoreEvent::Failed`。

### 数据变动

- 不动索引 schema。
- core `claude/history.rs`：新增 `pub struct HistoryPrompt { pub ts: i64, pub text: String }` 和 `HistoryGroup`（id、是否有真实 id、行）。
- app `SessionRow`：删 `resume`；加 `files_touched: Vec<SharedString>`（已按项目目录转成相对路径，排好序）和 `files_fmt`（"12 files" / "—"）。
- app `settings.rs` 不变：导出目录只在本次运行里记住，存在 `AppRoot` 的字段里。

### 逻辑变动

#### core

- **`transcript.rs`**：`read_transcript` 遇到打断标记的 user 记录直接跳过（判断函数和 conversation 共用，放在 `transcript.rs` 或 `conversation.rs` 里 `pub(crate)`）。被打断的回合按现有规则输出：有回复写回复，没有写 `status="missing"`。更新 `kb/notes/2026-10-10-transcript-reader-xml.md` 里相应的一句。
- **`claude/history.rs`**：抽出 `group_history`；新增 `history_prompts`（对每条 prompt 做 `expand_pasted` + `clean_prompt`，和 `history_sessions` 取 first / last prompt 的方式一致）。
- **`claude/conversation.rs`**：新增 `pub fn conversation_from_history(prompts: &[HistoryPrompt], matcher: Option<&Matcher>) -> Conversation`。

#### app

- **`actions.rs`（新）**：
  - `pub struct SessionActions { favorite, copy_resume, copy_id, reveal, open_folder, export }`，每项是 `Result<(), &'static str>`（不可用时带英文原因，就是 PRD 第 1 节表里的提示）；`resume: Option<String>`。
  - `pub fn available(row: &SessionRow, original_exists: bool, dir_exists: bool) -> SessionActions`：纯函数，按 PRD 的表判断。
  - `pub fn check(row: &SessionRow, claude: &ClaudeDir) -> SessionActions`：做 stat 后调 `available`。只有收藏的行才查原文件。
  - `pub fn resume_command(id: &str, project_path: &str, dir_exists: bool) -> Option<String>`：路径里有空格或 shell 特殊字符时加单引号。
  - `pub fn export_name(row: &SessionRow) -> String`：`<项目名>-<id 前 8 位>.txt`，项目名里的 `/` 等换成 `-`。
  - action 定义：`CopySessionId`、`CopyResume`、`RevealTranscript`、`OpenProjectFolder`、`ExportTranscript`、`HideProject`，都是 `{ id: Option<String> }`；`impl AppRoot` 里各一个处理函数：取 id 对应的行（`None` 用选中行）→ `check` → 不可用就 `push_notification(原因)`，可用就执行。复制用 `cx.write_to_clipboard(ClipboardItem::new_string(..))`。
- **`main.rs`**：
  - `bind_keys`：`cmd-c` → `CopySessionId { id: None }`、`cmd-shift-c` → `CopyResume { id: None }`、`[` → `PrevTurn`、`]` → `NextTurn`，都在 `"DataTable"` 上下文。
  - `render` 里加这些 action 的 `on_action`。
  - `select_row` 时 `self.detail.actions = actions::check(&row, &self.claude)`，标题行和 Metadata 读它。
  - 换成带额外图标的 `AssetSource`（`square-terminal`、`file-search`、`download`），照 gpui-kit-assets README 的写法。
  - `AppRoot` 加 `last_export_dir: Option<PathBuf>`。
- **`detail.rs`**：
  - `render_detail_header`：标题右侧换成按钮组（★ 在最前），用 `Button` 的 `.tooltip(..)` 和 `.disabled(..)`；尺寸、间距照画稿 1 的 `.btn` / `.ibtn`（22px 高，图标按钮 22×22）。
  - `render_metadata`：Resume 一行用 `detail.actions.resume`；Top tools 下面加 Files（前 10 个 + Show all）。
  - `DetailState` 加 `actions: SessionActions`；加 `step_turn(forward)`：在行表里找当前视口顶行之后（或之前）的下一个 `Row::Head`，`scroll_to` 过去，不改展开状态。
- **`table.rs`**：`SessionTableDelegate` 创建时带上 `ClaudeDir`；`context_menu` 按 PRD 第 2 节重写：每项派发带 `id: Some(..)` 的 action，`disabled` 来自 `actions::check`；"Copy Resume Command"和"Copy Session ID"用 `PopupMenuItem::element` 画两行（命令预览）和右侧的快捷键提示。原来的中文"收藏 / 取消收藏"改为"Favorite" / "Unfavorite"，"在 Finder 中显示"由 `RevealTranscript` 取代。
- **`store.rs`**：`ReadRequest` 改成枚举（决策 4）；`Store::read_history(session_id, project_key, matcher) -> u64` 和 `Store::export(src, dest)`；读取线程需要 `ClaudeDir` 来找 `history.jsonl` 和 `paste-cache`。新事件 `StoreEvent::Exported { dest, skipped_lines }`。History 的结果仍走 `StoreEvent::Conversation`。
- **`main.rs::select_row`**：History session（`transcript_path` 为 `None`）改为发 `read_history`；`conversation_note` 里 History 的那句说明删掉，由顶栏下面的灰字取代。
- **`conversation.rs`（app）**：
  - `segments(text)`（决策 6）和 `fold` 配合：折叠处落在代码块中间时，折到代码块结束前的那一行。
  - `build` 接受 History 的 `Conversation`，产出不带状态灯、刻度和回复的卡片，顶栏写"N prompts from history.jsonl"。
  - 新函数 `text_runs(text, segment, hits, current, base_font, mono_font) -> Vec<TextRun>`。
- **`turn_cards.rs`**：prompt、过程文字、回复按段渲染；代码块用新的样式 `style::code_block_bg()`（#f7f7f9）和 `style::inline_code_bg()`（#ededf0）；顶栏在没有搜索时显示 `[` `]` 键帽提示。
- **`model.rs`**：`from_summary` 填 `files_touched` / `files_fmt`；删 `resume`。

### 实施步骤

每一步先写行为测试再实现（BDD）。界面的样式不写行为测试。

- [ ] **步骤 0：验证 resume 的两个前提**（见"风险与待定"第 1、2 条）。先查 `claude --help` 和 Claude Code 文档；需要实际跑 `claude --resume` 时，先问用户（会在 `~/.claude` 里留下记录）。结论写进 session 总结，并据此定下"项目目录不存在"一行的行为，必要时回来改 PRD 第 1 节。
- [ ] **步骤 1：core**
  - 测试：
    - `crates/core/tests/transcript_reader.rs`：`an_interruption_marker_is_not_exported_as_user_input`（`reader.jsonl` 里加一条打断标记；同时检查原有的期望输出不变）
    - `crates/core/tests/history.rs`：`history_prompts_of_a_session_are_its_rows_with_pasted_text_restored`、`an_anonymous_history_session_gets_the_prompts_of_its_time_group`、`history_sessions_and_history_prompts_group_the_same_way`
    - `crates/core/tests/conversation.rs`：`a_history_conversation_is_one_prompt_only_turn_per_prompt_with_hits_marked`
  - 完成标准：`cargo test -p agent-session-core`；`make parity` 21/21（`history_sessions` 改了内部结构，parity 会覆盖到）。
- [ ] **步骤 2：`actions.rs`**
  - 测试：
    - `the_resume_command_changes_into_the_project_folder`
    - `the_resume_command_quotes_a_path_with_spaces`
    - `the_resume_command_without_a_project_folder_follows_step_0`（期望值按步骤 0 的结论写）
    - `a_history_session_can_copy_its_id_and_open_its_folder_only`
    - `a_session_without_an_id_can_only_open_its_folder`
    - `a_favorite_whose_original_is_gone_cannot_be_resumed_but_can_be_exported`
    - `a_missing_project_folder_disables_only_open_folder`
    - `the_export_name_is_project_and_short_id`
- [ ] **步骤 3：store**
  - 测试（`store.rs`）：`history_prompts_are_served_for_a_history_session`（fixture 里有 4 个 History session）、`an_export_writes_the_file_and_reports_skipped_lines`、`an_export_is_not_superseded_by_a_conversation_request`。
- [ ] **步骤 4：快捷键与 action**
  - 测试（`keyboard_tests.rs`，无界面窗口用的是测试平台的剪贴板，不碰系统剪贴板）：
    - `command_c_copies_the_session_id_and_shift_command_c_the_resume_command`
    - `command_c_in_the_search_box_does_not_copy_the_session_id`
    - `copying_the_resume_command_of_a_history_session_copies_nothing`
    - `brackets_jump_between_turns_and_stop_at_the_ends`
    - `brackets_do_nothing_on_the_metadata_tab`
- [ ] **步骤 5：分段**
  - 测试（app `conversation.rs`）：
    - `fenced_code_becomes_a_code_segment_and_an_unclosed_fence_runs_to_the_end`
    - `inline_code_is_found_in_text_segments_only`
    - `folding_counts_lines_across_segments_and_does_not_cut_inside_a_code_block`
    - `inline_code_and_hits_combine_into_runs_with_the_hit_on_top`
- [ ] **步骤 6：界面**：标题行按钮组与图标、右键菜单、Metadata 的 Files、History 卡片、`[ ]` 提示、代码块渲染。`model.rs` 的测试 `a_row_shows_the_project_folder_name_and_a_short_id` 删掉对 `resume` 的断言，新增 `a_row_lists_the_files_it_touched_relative_to_the_project`。
- [ ] **步骤 7：验收与收尾**
  - autoplay 加三步（接在 M4a 的 `ASM_DETAIL_AUTOPLAY` 后面）：`history-session`（选第一个 History 行）、`code-block`（选 `ASM_AUTOPLAY_SESSION`，展开一个回复里有代码块的回合）、`next-turn`（模拟 `]` 两次）。不在 autoplay 里复制。
  - 右键菜单用 cliclick 鼠标点击打开（AGENTS：只用于无法程序化打开的弹出层），加到 `screenshot-clicks.sh`；悬停提示人工看一次。
  - 截图归档到 `tmp/<date>-m4b-actions/`，逐张用 `Read` 看，和画稿 1、2、5、6 对照。
  - 恢复命令的端到端验证：请用户从菜单复制一次、粘到终端里执行（验收时不能自己动剪贴板）。
  - 导出：用界面导出一个 session，和 `make transcript-reader ARGS="<id> -o tmp/…"` 的结果 `diff`。
  - `cargo test --workspace`、`make parity`。
  - 文档按总 plan 的清单更新；`kb/docs/product-prd.md` 的"精简 TXT 导出"一行改成 GUI 已完成；known-issues 删掉"导出把打断标记当成用户输入"那条。
  - 勾选总 plan 的 M4b 一步，提交。

### 测试与验证

| 验收标准 | 怎么验 |
|---|---|
| 1 标题行按钮 | 截图 `conversation`，对照画稿 1 |
| 2 恢复命令可用 | 步骤 2 的命令测试；用户在终端里实际执行一次 |
| 3 Finder / 项目目录 / 复制 ID | 人工点一次；复制 ID 由步骤 4 的键盘测试覆盖 |
| 4 置灰 | 步骤 2 的可用性测试；截图 `history-session` 对照画稿 5 |
| 5 右键菜单 | `screenshot-clicks.sh` 截图，对照画稿 6 |
| 6 快捷键 | 步骤 4 的键盘测试 |
| 7 导出 | 步骤 1 的导出测试、步骤 3 的 store 测试；和 `transcript-reader` 的结果 `diff` |
| 8 History 对话 | 步骤 1、3 的 History 测试；截图 `history-session` |
| 9 修改过的文件 | 步骤 6 的 `model.rs` 测试；截图 Metadata |
| 10 按回合跳转 | 步骤 4 的键盘测试；截图 `next-turn` |
| 11 代码块 | 步骤 5 的测试；截图 `code-block` 对照画稿 2 |

### 风险与待定

1. **`cd` 进项目目录的必要性，以及目录不存在时怎么办**（D8，未验证）。推测 `claude --resume <id>` 只在当前目录对应的 `~/.claude/projects/<编码后的路径>/` 下找 session，所以要先 `cd`。如果属实，项目目录不存在时去掉 `cd` 也恢复不了，这时 Copy resume 应置灰，提示"The project folder no longer exists"，而不是退回不带 `cd` 的命令。步骤 0 验证后定。
2. **收藏但原文件已被清理的 session**：推测 Claude Code 只在 `~/.claude/projects` 下找，收藏副本不在那里，所以置灰。如果步骤 0 发现能指定文件路径恢复，再回来讨论。
3. **菜单里带参数 action 的快捷键提示**：按决策 1 用 `PopupMenuItem::element` 自己画。如果自定义项不支持 `disabled` 或点击后不关闭菜单，就退回普通的 `menu_with_disabled`，不显示快捷键提示，在 session 总结里记一笔。
4. **`TextRun` 拼接的边界**：行内代码和命中区间按字节切分，必须落在字符边界上，`text_runs` 的测试要包含中文。
5. **导出文件写到用户选的位置**：同名文件由系统保存对话框询问是否替换，应用不再二次确认。
