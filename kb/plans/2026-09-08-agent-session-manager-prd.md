---
created: 2026-09-08
tags:
  - prd
  - spec
  - gpui
  - agent-session-manager
  - plan
---

# Agent Session Manager PRD

一个用 GPUI（Rust）编写的 macOS 桌面应用，以表格形式列出本机所有 Claude Code session，支持按项目 / 时间过滤、表头排序、按消息角色范围的全文搜索、可切换的行展示方式，以及选中 session 的详情面板。核心诉求是**界面响应速度与数据查询速度**。

本 PRD 从 `~/Code/scripts/claude_session_search.py` 已有能力出发整理并补充需求，每条需求下写明大致实现方式。技术依据见：

- `kb/docs/claude-session-data-format.md`：数据源、记录类型、派生字段
- `kb/docs/gpui-framework-research.md`：GPUI / gpui-kit 的用法、限制与最佳实践
- `kb/docs/data-layer-design.md`：SQLite FTS5 索引、解析、增量更新、搜索模式

## 0. 背景与目标

现有 Python 脚本已能做两件事：（a）按正则 / 文本在最近 N 天的 transcript 里搜索，可限定角色（user / assistant / tool）、标签（`tool_use:Bash`、`tool_result`、`thinking`）、项目子串，输出带上下文的高亮片段；（b）`--list` 列出某项目的 session（ID、首条 prompt、prompt 数、大小、更新时间），transcript 被清理后从 `history.jsonl` 恢复。痛点：每次全量扫描 900MB、无法交互式筛选和排序、看不到消息全文、多个项目之间切换要重复跑命令。

目标：

| 指标 | 目标值 |
|---|---|
| 热启动到表格可交互 | < 300ms（索引已存在时只读 SQLite） |
| 冷启动首次全量索引 917MB | < 3s，且期间 UI 可用（进度条 + 已索引的行先显示） |
| 过滤 / 排序响应 | < 16ms（一帧内），几千行在主线程直接算 |
| 全文搜索返回 | < 100ms（FTS5），正则模式 < 500ms（热缓存） |
| 表格滚动 | 60fps，任意行数 |
| 内存 | < 150MB 常驻（不把 tool 输出读进内存） |

非目标（一期）：编辑 / 删除 transcript、启动新 session、云同步、Windows / Linux 打包、Codex 等其他 agent（仅在数据模型里预留 `agent` 字段）。

用户：本人；单机；数据量参考本机 85 项目 / 654 主 session / 242 subagent / 917MB。

## 1. 需求与实现方式

### R1 Session 表格（主视图）

**需求**：所有 session 以表格展示，一行一个 session；默认按更新时间倒序；虚拟化滚动；行可选中、双击打开详情、右键菜单；列宽可拖拽；列可显示 / 隐藏；支持键盘上下导航。

默认列（按顺序）：

| 列 | 内容 | 来源 | 可排序 |
|---|---|---|---|
| Project | 项目名（路径末段），hover 显示完整路径；worktree 项目显示仓库名 + 分支 | `sessions.cwd` / `projects.path` | 是 |
| Preview | 由 R5 展示模式决定：标题 / 首条 prompt / 最后 prompt / 最后 assistant 消息，两行布局 | `title` / `first_prompt` / `last_prompt` / `last_message` | 否 |
| Started | 首条人类消息时间，`MM-DD HH:mm`，hover 完整 | `started_at` | 是 |
| Updated | 最后活动时间；1 分钟内更新显示"live"绿点 | `updated_at` | 是（默认） |
| Prompts | 人类消息数 | `user_msg_count` | 是 |
| Size | 人类可读大小 | `file_size` | 是 |
| Cost | `$1.97`，无 cost-state 时空 | `total_cost_usd` | 是 |
| Branch | git 分支 | `git_branch` | 是 |
| Tools | tool_use 数；hover 显示 top 5 工具名 | `tool_use_count` / `session_tool_counts` | 是 |
| Kind | 来源徽章：CLI（默认不加标记）/ **SDK**（`entrypoint = sdk-cli`）/ **BG**（`sessionKind = bg`）/ **History**（transcript 已被清理）；另有 subagent 数与 PR / Artifact 图标 | `entrypoint` / `is_background` / `source` / `subagent_count` / `pr_url` | 是（按 kind 分组） |
| ID | 短 uuid（前 8 位），点击复制 | `id` | 否 |

**实现**：
- gpui-kit `DataTable` + `TableState<SessionTableDelegate>`。delegate 持有 `store: Arc<SessionStore>`（全量行，`Vec<Arc<SessionRow>>`，字段预格式化为 `SharedString`）和 `rows: Vec<usize>`（过滤 + 排序后的索引向量）。`rows_count()` 返回 `rows.len()`，`render_td` 通过 `rows[row_ix]` 取行。
- 行高 `with_size(px(48.))`，Preview 列渲染两行：第一行 `title`（truncate、正常色），第二行预览文本（truncate、`muted_foreground`）。
- `TableEvent::SelectRow(ix)` → 更新 `AppRoot.selected` → 详情面板刷新；`DoubleClickedRow` → 打开右侧 Sheet 或聚焦详情；`RightClickedRow` → `context_menu` 返回 `PopupMenu`（复制 ID、复制 resume 命令、在 Finder 显示、打开项目目录、在终端 resume）。
- 列显示 / 隐藏与列宽持久化到设置文件（`ColumnWidthsChanged` 事件）。
- 键盘：DataTable 内置上下 / home / end / pageup；额外绑定 `enter` 打开详情、`cmd-c` 复制 ID、`cmd-f` 聚焦搜索框（见 R10）。

### R2 过滤区域（顶部工具栏）

**需求**：位于表格上方一行。

1. **项目过滤**：可搜索的下拉，多选；列表按最近活动排序，显示每项目 session 数；"全部"为默认。同一 git 仓库的 worktree 目录（`~/.herdr/worktrees/<repo>-<branch>` 等）在下拉里按仓库聚合，可展开选具体 worktree。
2. **时间过滤**：预设（今天 / 3 天 / 7 天 / 30 天 / 全部）+ 自定义日期范围；作用字段可切换 Updated（默认）或 Started。
3. **更多过滤（折叠在 "Filters" 弹出面板）**：Kind 三态多选（CLI / SDK / BG，**默认全选，不隐藏任何一种**，仅靠 Kind 列徽章区分）、来源（transcript / history-only）、最少 prompt 数（默认 ≥1，过滤掉空 session）、权限模式、使用过的工具名（多选）、有 PR / 有 subagent。
4. 当前生效的过滤以 chip 显示，可逐个移除；"Reset" 清空。
5. 过滤条件持久化，下次启动恢复。

**实现**：
- 项目：`Select`（`searchable(true)`）或 `Combobox`（`multiple(true)`），自定义 `SelectItem` 携带 `project_id` 与计数。项目列表来自 `SELECT project_id, count(*), max(updated_at) FROM sessions GROUP BY 1`。
- 时间：`TabBar::segmented` 做预设 + `DatePicker`（`DatePickerState::range`）做自定义；`DateRangePreset`。
- 过滤逻辑统一为 `FilterSpec` 结构体（serde 持久化）。**过滤在内存中做**：`SessionStore` 已把所有 session 行（几千行、每行 <1KB）常驻内存，`apply(&FilterSpec) -> Vec<usize>` 是单次线性扫描（几千行 <1ms），直接在主线程执行后 `table.update(.. rows = idx; cx.notify())`。只有当搜索（R4）生效时才需要查 SQLite。
- 所有输入事件（`SelectEvent` / `DatePickerEvent` / `InputEvent::Change`）汇聚到 `AppRoot::on_filter_changed`，防抖 50ms（Task 重赋值即取消上一个）后重算。

### R3 表头排序

**需求**：Started / Updated / Prompts / Size / Cost / Branch / Tools / Project 可点击排序，三态（升 / 降 / 默认）；默认 Updated 降序；排序状态持久化；稳定排序（同值按 updated 降序）。

**实现**：
- `Column::new(key, name).sortable()`；`TableDelegate::perform_sort(col_ix, ColumnSort, ..)` 里对 `rows` 原地 `sort_by`，比较器按 `key` 分派到 `SessionRow` 的预解析字段（时间用 `i64` ms，大小用 `u64`，字符串用 `Ord`），`ColumnSort::Default` 恢复到 updated 降序。
- 排序与过滤合并为 `recompute_rows()`：`filter → sort`，任一变化都重跑；几千行排序 <1ms。
- 排序状态存入 `ViewState { sort_key, sort_dir }` 持久化。

### R4 搜索

**需求**：

1. 工具栏搜索框，`cmd-f` 聚焦，`esc` 清空并回到表格。
2. **范围选择**（分段控件）：User（人类消息）/ Assistant（助手正文）/ Both（默认）/ Tools（tool_use 输入与 tool_result，仅正则模式可用）。对应脚本的 `-r user -r assistant -r tool`。
3. **模式**：文本（默认，FTS5，大小写不敏感，词组 AND）/ 正则（开关 `.*`，对应脚本的 regex 查询，`(?i)` 可切换大小写敏感）。
4. **标签过滤**（正则模式下）：`tool_use:Bash`、`tool_result`、`thinking` 等，对应脚本 `-l`。
5. 搜索生效时：表格只显示有命中的 session，新增 "Matches" 列显示命中数并可排序；Preview 列改为显示**第一个命中片段**（带高亮），对应脚本的 `-C` 上下文片段。
6. 详情面板（R6）内命中高亮，并提供上一个 / 下一个命中跳转。
7. 搜索与 R2 的过滤条件叠加（先按项目 / 时间缩小范围再搜）。
8. 结果上限 200 个 session，超出提示；搜索耗时显示在状态栏。

**实现**：
- 文本模式：SQLite FTS5 查询（`kb/docs/data-layer-design.md` §2.1），`role` / `project_id` / `ts` 在基表过滤；`GROUP BY session_id` 得到命中数与 `min(bm25)`，`snippet()` 取每 session 首个片段。查询在**后台线程**（SQLite 连接线程）执行，结果经 channel 回前台。1–2 字 CJK 查询自动改走 `LIKE` 路径。
- 正则模式：`grep-searcher` + `grep-regex` 扫 SQLite 按项目 / 时间预过滤后的文件列表，`rayon` 并行；每行先字节预判角色（`"type":"user"` / `"type":"assistant"`）再 JSON 解析出 block；Tools 范围复用脚本 `iter_texts` 的逻辑（tool_use 的 `k: v` 拼接、tool_result 的 text 拼接）与 `render_tool_use` 的展示格式（description `#`、command `$`）。结果流式回前台，每批 notify。
- 高亮：FTS 的 `snippet()` 用 `char(1)`/`char(2)` 标记切成区间；正则用 `find_iter` 的字节偏移；渲染为 `StyledText::new(text).with_highlights(ranges)`。
- 搜索输入防抖 150ms；新任务替换旧 `Task` 即取消；正在跑的正则扫描通过 `AtomicBool` 取消。
- `clean_prompt` / `human_prompt_text` 逻辑移植到 Rust（去 `<system-reminder>` 等包裹块、`isMeta`、`[Image: source:` 判断）。

### R5 展示方式调整

**需求**：

1. Preview 列的内容模式（分段控件或菜单）：**Title**（custom-title > agent-name > ai-title > 首条 prompt 首行）/ **First prompt**（默认）/ **Last prompt** / **Last assistant message**。切换即时生效，无需重新查询。
2. 行密度：Compact（单行 32px）/ Comfortable（两行 48px）。
3. 时间显示：相对（"3h ago"）/ 绝对。
4. 列显示 / 隐藏、列顺序（拖拽）。
5. 以上均持久化。

**实现**：
- `SessionRow` 同时预存 `title / first_prompt / last_prompt / last_message` 的单行化版本（换行替换为 `⏎`，截断到 240 字，对应脚本 `PROMPT_LINE_LEN`）；`render_td` 根据 `DisplayMode` 选字段，切换只需 `cx.notify()`。
- 行密度通过 `DataTable::with_size(..)` 切换；列显隐通过重建 `columns: Vec<Column>` 后 `table.refresh(cx)`；列顺序用 `TableDelegate::move_column` + `MoveColumn` 事件持久化。
- 设置结构 `ViewState` 用 serde JSON 存到 `~/Library/Application Support/AgentSessionManager/state.json`。

### R6 Session 详情面板

**需求**：

1. 右侧可拖拽分栏（默认 40% 宽，可折叠）；选中行即显示。
2. 头部：标题、项目路径、session ID（点击复制）、started / updated、分支、prompts / messages / tools 计数、cost 与 token 用量、时长、使用的模型、权限模式、PR / Artifact 链接、修改过的文件列表（折叠）。
3. 操作按钮：复制 `claude --resume <id>`、在终端中打开（`cd <cwd> && claude --resume <id>`）、在 Finder 显示 transcript、打开项目目录、导出 Markdown。
4. 消息流：按时间顺序显示 user / assistant 消息，assistant 正文 Markdown 渲染，长消息默认折叠 5 行可展开；tool_use 显示为可折叠卡片（工具名 + description + command，对应脚本 `render_tool_use`）；tool_result 默认折叠仅显示前 3 行与字节数；thinking 默认隐藏。顶部有角色过滤 chip（User / Assistant / Tools / Thinking）。
5. 搜索命中时定位到首个命中并高亮，`n` / `shift-n` 跳转。
6. subagent：若有 `subagents/`，在头部显示列表（agentType、description），点击在面板内切换查看该 subagent 的消息流。
7. history-only session：显示 history.jsonl 里的 prompt 列表并提示 transcript 已被清理。

**实现**（2026-09-28 按 M4 方案 `2026-09-28-m4-detail-panel.md` 修订；标"待确认"的条目对应该方案第 1 节的 D1–D6，用户尚未拍板，这里写的是方案的默认选择）：
- 布局 `h_resizable("main")` 两个 `resizable_panel`；折叠用 `.visible(false)`。
- 消息内容**不进 SQLite**，选中时后台线程按需 mmap 原文件、解析该 session 的全部 user / assistant 记录，把 tool_use 与 tool_result 按 `tool_use_id` 配对；每条文本与 tool_result 按上限截断并保留原始字节数 / 行数。读取放在 core（`claude/transcript.rs`），经 `store.rs` 内部的第二条线程进出，不排在索引扫描后面。本机最大单文件已是 70MB（2026-09-28 实测，完整解码一遍 ≤ 117ms），不再是立项时的 30MB。
- 消息流用 GPUI 核心 `list` + `ListState`（**待确认 D1**）。gpui-kit 的 `v_virtual_list` 要求预先给出每一项的高度，自动换行的消息给不出来。
- Markdown 用 `TextView::markdown(id, src).max_lines(5)`，`is_clamped()` 决定是否显示"展开"。`TextView` 没有传入搜索高亮的接口，所以搜索时有命中的消息显示带高亮的纯文本，其余显示 Markdown（**待确认 D3**）。`max_lines` 要先布局整篇文档再裁剪，超长正文需先截断。
- 详情内的命中高亮由 core 的 `Matcher` 在读取时计算，覆盖工具输入 / 输出与 thinking；范围不受 Scope / Labels 限制（**待确认 D5**）。
- Shell 命令用 `shfmt` 重排（若 PATH 有，对应脚本 `format_command`）。每条命令起一个进程，所以只在展开工具卡片时才格式化，不在读取时做。
- 导出 Markdown 复用解析结果，格式沿用 `~/Code/agent-sessions` 的中间格式思路。
- "在终端中打开"不在 M4 内，M4 只做复制命令（**待确认 D6**，同 §5 开放问题 2）。

### R7 索引与实时更新

**需求**：

1. 首次启动全量索引，状态栏显示进度（已扫描文件 / 总数、耗时），表格随索引流式填充。
2. 之后启动只做 stat 扫描 + 尾部增量解析，通常 <100ms。
3. 运行期间监听 `~/.claude/projects` 与 `history.jsonl`，新 session / 新消息自动出现，Updated 列与 live 指示实时刷新（防抖 500ms）。
4. transcript 被 Claude Code 清理后，行保留并标记 history-only。
5. 菜单 "Rebuild Index"；schema 版本变化自动重建。
6. 索引损坏（打开失败）时删除重建，不崩溃。

**实现**：`kb/docs/data-layer-design.md` §3–§4、§6。核心 crate 提供 `Indexer { scan_all(progress_tx) / scan_paths(paths) / rebuild() }`，运行在专用线程；进度与增量结果通过 `smol::channel` 进前台 `cx.spawn` 任务，按块 `cx.notify()`。`notify-debouncer-full` 事件与 30s 轮询兜底。

### R8 history.jsonl 回退与 sessions-index.json 补充

**需求**：沿用脚本的 `merge_sessions` 语义：transcript 优先，history 补位；history 无 `sessionId` 的老记录按 2 小时间隔切分为匿名 session（显示为 "(unknown session id)"）。`sessions-index.json` 存在时用其 `summary` 作为标题候选。

**实现**：`history_prompts` 表增量导入；启动时对 `sessions` 缺失的 `sessionId` 生成 `source = 'history'` 行；匿名分组逻辑移植 `history_sessions()`。`sessions-index.json` 仅读取不依赖。

### R9 Subagent session

**需求**：默认不作为独立行出现（避免 242 行噪音），主 session 的 Flags 列显示 subagent 数；详情面板可查看（R6.6）；设置里可切换"把 subagent 显示为子行"（缩进显示在父 session 下方）与"搜索包含 subagent"。

**实现**：索引时 `parent_session_id` / `is_sidechain = 1`；FTS trigger 默认排除 sidechain，开关打开时重建 FTS 或改用查询期过滤。子行模式下 `rows` 索引向量插入子行并在 `render_td` 缩进。

### R10 键盘与快捷键

| 键 | 动作 |
|---|---|
| `cmd-f` | 聚焦搜索框 |
| `esc` | 清空搜索 / 关闭面板 / 回到表格 |
| `↑ ↓ / j k` | 选行 |
| `enter` | 打开 / 聚焦详情 |
| `cmd-c` | 复制 session ID |
| `cmd-shift-c` | 复制 resume 命令 |
| `cmd-r` | 在终端 resume |
| `cmd-1..4` | 切换 Preview 模式 |
| `cmd-\` | 折叠 / 展开详情面板 |
| `cmd-,` | 设置 |
| `cmd-shift-r` | Rebuild index |

**实现**：`gpui_kit::actions!(app, [...])` + `cx.bind_keys` + 根 div `key_context("app")` / 表格 `key_context("session_table")`；菜单项引用同一批 Action 自动显示快捷键。

### R11 设置

Claude 数据目录（默认 `~/.claude`，可改以支持多账号）、主题（跟随系统 / 亮 / 暗）、终端应用（Terminal / iTerm / Ghostty / Kitty，生成对应 `open` 或 AppleScript）、默认过滤、是否索引 subagent、`cleanupPeriodDays` 提示。存 `state.json`。

### R12 状态栏

左：当前显示 N / 总 M 个 session，过滤 chip；中：索引状态（idle / scanning 123/896 / watching）；右：最近一次搜索耗时、DB 大小。

### R13 扩展性（不实现，只留接口）

核心 crate 定义 `trait SessionSource { fn scan(&self) -> …; fn parse(&self, path) -> …; fn agent(&self) -> &str }`，一期只有 `ClaudeCodeSource`。`projects.agent` 字段预留。Codex（`~/.codex/sessions`，本机 144 份）与 `~/Code/agent-sessions` 的 parser 是二期候选。

## 2. 架构

```text
agent-session-manager/            Cargo workspace
├── crates/core/                  agent-session-core：无 gpui 依赖
│   ├── model.rs                  SessionRow / MessageView / FilterSpec / DisplayMode（serde）
│   ├── claude/{parse.rs,clean.rs,history.rs}   JSONL 借用解析、clean_prompt、history 分组
│   ├── index/{db.rs,schema.sql,indexer.rs,watch.rs}   SQLite、增量、FSEvents
│   ├── search/{fts.rs,regex.rs,highlight.rs}   FTS5 查询构造、grep-searcher、区间合并
│   └── bin/asm.rs                CLI：index / list / search / stats（替代 Python 脚本，也是数据层的测试入口）
├── crates/app/                   agent-session-manager：gpui-kit UI
│   ├── main.rs                   application().run、菜单、快捷键、窗口尺寸恢复
│   ├── app_root.rs               AppRoot：工具栏 + 分栏 + 状态栏，状态汇聚
│   ├── table.rs                  SessionTableDelegate
│   ├── toolbar.rs                过滤 / 搜索 / 展示模式控件
│   ├── detail.rs                 详情面板与消息流
│   ├── store.rs                  SessionStore：内存行集 + 与 core 的通道
│   └── settings.rs               ViewState 持久化
├── kb/                           知识库（本目录）
└── AGENTS.md
```

线程模型：主线程（GPUI 前台）、SQLite 线程（单连接，closure 队列）、索引线程池（rayon）、watcher 线程。所有跨线程数据用 `Arc<…>` 不可变快照 + `smol::channel`。

依赖（锁定精确版本）：`gpui-kit = "=0.6.1"`（内含 gpui-pre 0.3.x = Zed main 快照；选型论证见 `kb/docs/gpui-framework-research.md` §1.3）、`rusqlite` bundled、`serde` / `serde_json`（raw_value）、`memmap2`、`memchr`、`rayon`、`notify` + `notify-debouncer-full`、`grep-searcher` + `grep-regex`、`regex`、`jiff`（时间）、`directories`、`anyhow`、`smol`、`tracing`。

**不要自己再声明 `gpui` 依赖**，否则会出现两份 GPUI 导致类型不匹配；核心 crate `crates/core` 完全不依赖 gpui，UI 层若需退回官方 git 依赖可独立替换。

## 3. 里程碑

| 阶段 | 内容 | 验收 |
|---|---|---|
| **M0 环境与 spike**（0.5 天）✅ 2026-09-15 | rustup 最新 stable、Metal Toolchain；`gpui-kit` hello world + `DataTable` 10k 假行；确认 `h_resizable` / `TabBar::segmented` / `DatePicker` 签名；记录构建时间 | 窗口能开、表格能滚、截图归档（记录：`../notes/2026-09-15-m0-spike.md`） |
| **M1 核心 crate**（2 天）✅ 2026-09-17 | 解析器（fixture JSONL 覆盖全部记录类型）、SQLite schema 与索引器、增量逻辑、FTS 与 LIKE 搜索、正则扫描、`asm` CLI | BDD 测试：`asm list` 与 Python 脚本 `--list` 输出一致；`asm search` 命中数与脚本一致；全量索引本机 <3s（记录：`../notes/2026-09-17-m1-core-crate.md`，61 个场景 + 21 组真实数据逐字比对通过，全量 2.1–2.4s） |
| **M2 表格 + 过滤 + 排序**（2 天）✅ 2026-09-21 | R1 R2 R3 R5 R12，SessionStore 流式填充 | 冷启动可用、热启动 <300ms、过滤排序一帧内（记录：`../notes/2026-09-21-m2-real-data.md`；热启动 ~200ms 上屏 5488 行、冷索引 3.6–4.0s 流式填充、filter + sort 0–1ms。R1 右键菜单 / 列显隐、R2 多选 / 可搜索 / 自定义区间 / 更多过滤（其中 Kind 多选已于同日补上）、R5 行密度 / 相对时间未做，见笔记"未做"一节） |
| **M3 搜索**（1.5 天）✅ 2026-09-21 | R4 全部，高亮，Matches 列 | FTS <100ms、正则 <500ms、CJK 两字可搜（记录：`../notes/2026-09-21-m3-search.md`；文本 2–40ms、正则热缓存 240–425ms、两字 CJK 走 LIKE 在 debug app 里 83–137ms / release 14–27ms。正则用 M1 的 memmap 扫描而非 grep-searcher；详情面板里工具调用内的命中要等 M4） |
| **M4 详情面板**（2 天）方案已定稿，未开工 | R6，Markdown、折叠、subagent、导出。方案：`2026-09-28-m4-detail-panel.md`，拆成 M4a（直接读 transcript、虚拟化消息流、工具 / thinking 内的命中）与 M4b（工具卡片格式、头部与操作按钮、subagent、Markdown、history-only、导出），拆分本身**待确认 D2** | 立项口径：30MB session 打开 <200ms。方案建议改为"选中行到对话首帧上屏 <200ms，以本机最大的 transcript（现为 70MB）为基准"，**待确认 D4** |
| **M5 实时与打磨**（1.5 天） | R7 watcher、R8 history、R9、R10、R11、菜单、cargo-bundle、图标 | 新 session 5s 内出现；`.app` 可双击运行 |

开发规范：新功能先写行为测试（core crate 用 fixture JSONL；UI 用 gpui-kit 0.6.1 新增的 `gpui_kit::test` 无头 UI 测试 + `test-support` feature，覆盖 delegate 与交互）；每个里程碑完成后截图归档到 `tmp/<date>-<task>/` 并 commit。

## 4. 风险与对策

| 风险 | 对策 |
|---|---|
| gpui-kit / gpui-pre API 在开发期间 breaking | 精确锁版 + Cargo.lock；升级是独立任务；UI 层薄 |
| Xcode / rustc 版本导致编不过 | M0 首先解决；`runtime_shaders` 作为 Metal Toolchain 替代 |
| Claude Code transcript schema 变化 | 所有字段 Option、未知 type 忽略、fixture 测试覆盖已知类型、schema version 门控重建 |
| 单文件 30MB+ 的 session 详情打开慢（本机最大已到 70MB） | 按需解析 + tool_result 截断 + 后台线程 + 骨架屏。实测解析不是瓶颈（70MB ≤ 117ms，大在 base64 图片而非消息条数），预算主要花在首帧布局上 |
| trigram 对 2 字中文失效 | LIKE 回退（已设计） |
| FSEvents 丢事件 | 30s 轮询兜底 |
| `DataTable` 行高固定不够表达 | 两行 48px 设计；确需可变高再换 `v_virtual_list` |

## 5. 开放问题（需你拍板）

1. Preview 列默认用 **First prompt** 还是 **Title**（新版本才有 ai-title，覆盖率目前 5/654）？本 PRD 默认 First prompt。
2. 是否需要 "在终端 resume" 直接拉起终端（涉及 AppleScript 权限），还是仅复制命令？M4 方案默认仅复制命令，拉起终端留给 M5（D6）。
3. M4 方案的 6 个待确认项（虚拟列表选型、拆分交付、Markdown 与高亮、验收口径、详情内命中范围、在终端打开）：见 `2026-09-28-m4-detail-panel.md` 第 1 节，每项都写了前因、选项、默认与最晚确认的步骤。

### 已定决策（2026-09-15）

- **应用名 / bundle id**：Agent Session Manager / `com.reorx.agent-session-manager`（用户写作 `come.reorx.…`，按反向域名惯例取 `com.`，如需改回请说明）。
- **SDK session 不隐藏**：`entrypoint = sdk-cli` 与后台 session 一律显示，通过 Kind 列徽章区分，过滤器默认全选。

## 6. 相关文档

- [Claude Code Session 数据格式调研](../docs/claude-session-data-format.md)
- [GPUI 框架调研](../docs/gpui-framework-research.md)
- [数据层设计](../docs/data-layer-design.md)
- [M4 详情面板实现方案](2026-09-28-m4-detail-panel.md)
- 参考实现：`~/Code/scripts/claude_session_search.py`
- 旧项目：`~/Code/agent-sessions`（transcript → 静态站点 SSG，含 Codex parser）
