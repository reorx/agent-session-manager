---
created: 2026-09-28
tags:
  - m4
  - detail-panel
  - transcript
  - gpui
  - agent-session-manager
  - plan
---

# M4：详情面板（R6）实现方案

PRD 里程碑 M4，对应需求 R6（`2026-09-08-agent-session-manager-prd.md`）。基于 `m3` 分支（7ecdf60）继续。本方案写于开发之前，**第 1 节的待确认项尚未由用户拍板**：全文按每项的"默认"编写，用户改选时只需改动该项"影响"一栏列出的步骤。

必读：`CLAUDE.md`、`../notes/2026-09-21-m3-search.md`、`../notes/2026-09-21-m2-real-data.md`、`../docs/product-prd.md` §4.5。

## 1. 待确认项

状态一律为 **未确认**。开发到"最晚确认"所列的步骤之前必须得到答复；在那之前的步骤不受影响，可以先做。

| # | 问题 | 默认（本方案按此编写） | 最晚确认 |
|---|---|---|---|
| D1 | 消息流用哪种虚拟列表 | GPUI 核心 `list` + `ListState` | 步骤 3 之前 |
| D2 | M4 是否拆成两段交付 | 拆成 M4a / M4b，各自验收、各自提交 | 步骤 1 之前 |
| D3 | Markdown 渲染与命中高亮的冲突怎么处理 | 有命中的消息显示带高亮的纯文本，其余显示 Markdown | 步骤 9 之前 |
| D4 | 验收指标的口径 | 选中行到对话首帧上屏 < 200ms，基准为本机最大的 70MB transcript | 步骤 5 之前 |
| D5 | 详情面板的命中范围 | 高亮并跳转所有可见内容里的命中，不受 Scope / Labels 限制 | 步骤 4 之前 |
| D6 | "在终端打开"是否属于 M4 | 不属于，M4 只做复制命令；拉起终端留给 M5（R11 终端应用设置） | 步骤 7 之前 |

### D1 虚拟列表

- **前因**：PRD R6 与 M3 交接说明写的都是 gpui-kit 的 `v_virtual_list`。
- **事实**：`v_virtual_list(view, id, item_sizes: Rc<Vec<Size<Pixels>>>, f)` 要求预先给出每一项的高度（`gpui-base-0.6.1/src/virtual_list.rs:140`）。消息是自动换行的文本，高度随面板宽度变化，事先算不出来。
- **选项**
  - A（默认）`list` + `ListState`（`gpui-pre-0.3.5/src/elements/list.rs`）：自己测量高度；`scroll_to(ListOffset { item_ix, offset_in_item })` 可定位到一条消息内部；`remeasure_items` 对应展开 / 折叠；gpui-base 已实现 `ScrollbarHandle for ListState`；gpui-kit 的 `MessageScroller` 也基于它。
  - B `v_virtual_list`：必须自己估算每条消息的高度（按字符数和面板宽度），估错就会出现重叠或空白，面板宽度变化时要全部重算。
- **A 的代价**：视口外的项高度是估算值，拖动滚动条时滑块可能跳动。用 `reset_with_uniform_height(count, 22px)` 给出高度提示可以缓解（折叠的工具 / thinking 行正好是 22px，且数量最多）。
- **影响**：步骤 3、4。

### D2 拆分交付

- **前因**：PRD 给 M4 排了 2 天，但 R6 有 7 个条目，外加 M3 遗留的 3 项（过渡实现、命中跳转依赖、工具内命中不可见）。
- **选项**
  - A（默认）M4a = 步骤 1–5（数据通路、虚拟化、命中），M4b = 步骤 6–11（其余 R6 条目）。验收指标只取决于 M4a。
  - B 一次做完再验收。
- **影响**：提交粒度、文档更新时机、PRD 里程碑表的写法。

### D3 Markdown 与高亮

- **事实**：`TextView::markdown(id, text)` 没有传入高亮区间的接口（`gpui-component-0.6.1/src/text/compat.rs`）。`max_lines(n)` 需要先把整篇文档布局完再裁剪（`gpui-base-0.6.1/src/text/text_view.rs:537` 的注释），折叠并不省布局开销。
- **选项**
  - A（默认）搜索时有命中的消息用 `StyledText` + 高亮（即现在的渲染方式），其余用 Markdown。代价：同一条消息在搜索前后外观不同。
  - B 往源文本里注入 `<mark>`。gpui-base 的 HTML 解析支持 `<mark>`（`text/format/html.rs:363`），但 Markdown 里的内联 HTML 是否走这条路径**未验证**，且注入到代码块里会破坏内容。
  - C M4 不做 Markdown，记入 known-issues。
- **影响**：步骤 9。

### D4 验收口径

- **前因**：PRD 写的是"30MB session 打开 < 200ms"，当时本机最大文件是 30MB。
- **事实**：现在本机最大 transcript 是 70MB（第 3 节）。"打开"在 PRD 里没有定义起止点。
- **选项**
  - A（默认）起点 = `select_row`，终点 = 带对话内容的首帧绘制完成；基准文件 = 本机最大的 transcript（现为 70MB）；debug 构建（core 为 `opt-level = 3`）与 release 各测一次，以 release 为准。
  - B 维持 30MB 口径，70MB 只记录不作要求。
- **影响**：步骤 5 的验收脚本与 PRD / product-prd 的性能目标行。

### D5 命中范围

- **前因**：表格的 Matches 列遵循 Scope / Labels（正则模式按片段计数，文本模式按消息计数）。M3 的详情面板不看 Scope，对所有 user / assistant 消息做标记。
- **选项**
  - A（默认）详情里高亮所有可见内容的命中，类似页内查找。实现简单，但 `9 hits` 可能与 Matches 列的数字不同。
  - B 严格遵循 Scope / Labels，数字一致。需要把 `SearchOptions` 一并传给读取线程，条目要带上自己的标签。
- **影响**：步骤 2 的请求结构、步骤 4。

### D6 在终端打开

- **前因**：PRD §5 开放问题 2 至今未答复；拉起终端涉及 AppleScript 权限，且依赖 R11 的"终端应用"设置（M5）。
- **默认**：M4 只提供复制 `claude --resume <id>`。
- **影响**：步骤 7。

## 2. 现状（m3 分支，7ecdf60）

| R6 条目 | 现状 | 差距 |
|---|---|---|
| R6.1 分栏 | 已完成 | 无 |
| R6.2 头部 | 标题、id、项目、分支、Kind、计数 | token 用量、修改过的文件列表未显示（`files_touched` 已在索引里） |
| R6.3 操作按钮 | 无 | 全部未做 |
| R6.4 消息流 | 索引里的 user / assistant 纯文本，前 300 条 × 4000 字，不虚拟化 | 工具 / thinking 无数据，无 Markdown，无折叠 |
| R6.5 命中跳转 | 只覆盖 user / assistant | 工具 / thinking 内的命中看不到 |
| R6.6 subagent | 只有计数 | 列表与切换未做 |
| R6.7 history-only | 底部一句说明 | prompt 列表未做 |

影响方案的代码事实：

- `DetailState` 持有 `Vec<MessageRow>`（索引行），`model::conversation_from` 截 `CONVERSATION_CAP = 300` 条、`MESSAGE_CHARS = 4000` 字。为了让命中可见加了两个补丁：首个命中在 300 条之后时窗口前移（`skipped` / `skipped_turns`），命中在 4000 字之后时移动截断位置（`shown_text`）。
- 命中定位靠 `ScrollHandle::scroll_to_top_of_item(child_ix + lead_children)`，前提是每条消息都是滚动区的直接子元素（`detail.rs:123`）。
- `RawBlock` 没有 `id` / `tool_use_id` / `is_error` 字段（`parse.rs:518`）。tool_use 与 tool_result 在不同记录里，配对成一张卡片必须先加这些字段。
- `regex.rs::iter_texts` 是私有函数且面向搜索：把工具输入拍平成 `k: v` 文本。详情需要结构化数据，只能沿用它的拆分规则。`render_tool_use` 与 `format_command` 是 `pub`，可直接用。
- `detail.rs` 的 `render_thinking` / `render_tool` 与 `MessageKind::{Thinking, Tool}` 已存在，但不带高亮：`Message` 里只有 `text` 有 `highlights`，`input` / `output` 没有。
- store 线程是单循环，`Messages` 请求排在扫描后面：冷启动或 Rebuild 期间（3.6–4s）选中行，对话要等扫描结束才出现。
- history-only session 的 prompt 不在 `messages` 表里：`Indexer::import_history` 只写 `sessions` 行。要显示 prompt 列表得按需读 `history.jsonl`（`claude/history.rs::load_history`）。
- fixture 已覆盖 thinking（含只有 signature 的空 thinking）、tool_use、字符串与数组两种 tool_result、图片、subagent 及其 `meta.json`（`agentType` / `description` / `toolUseId` / `spawnDepth`）。

## 3. 实测（2026-09-28，本机）

| 项 | 数字 |
|---|---|
| 主 transcript 数量 | 1159 个；超过 5MB 的 80 个；超过 30MB 的 1 个 |
| 最大 transcript | 69,987,203 字节（70MB） |
| 该文件的构成 | 679 行；22 行超过 1MB；79 个 base64 图片；112 次工具调用；55 段 thinking |
| 其后几个 | 30.3MB、25.7MB、25.5MB、21.6MB |
| subagent | 106 个 session 带 `subagents/`，共 368 个 subagent transcript |
| 70MB 文件完整解码一遍 | 109–117ms |
| 25.7MB 文件所在项目（11 个文件共 35MB） | 97–104ms |

计时方法：`tmp/2026-09-28-m4-analysis/time-scan.sh`，用 release 版 `asm search` 以不会命中的正则扫描（含 `-r tool`，即解码每条 user / assistant 记录的全部 block）。数字含进程启动与计时脚本自身的开销，是上限。

结论：

1. 解析不是瓶颈。大文件大在图片和附件，不在消息条数；200ms 的预算主要花在界面首帧的布局上。
2. 每次请求重新读文件是可行的（见 4.3），不需要在内存里缓存解析结果。

## 4. 设计

### 4.1 数据通路

```text
select_row / 搜索变化
   │ StoreRequest::Transcript { session_id, path, matcher, seq }
   ▼
asm-transcript 线程（store.rs 内部的第二条线程，不持有 Db）
   core::claude::transcript::read(path, matcher)   mmap + classify + serde
   │ StoreEvent::Transcript { session_id, seq, transcript, elapsed }
   ▼
AppRoot::on_store_events → model::conversation_from(&transcript) → DetailState
```

- 对外仍是 `StoreRequest` / `StoreEvent`，"数据只经 `store.rs` 进出"的规则不变，事件 pump 不用改。`Store::request` 按请求类型分发到两条线程之一。
- 取代机制照搬 `search.rs`：`AtomicU64` 记最新 `seq`，线程开始解析前检查，过期请求直接丢弃；前台丢弃 `seq` 不符或 session 已切换的事件。
- `StoreRequest::Messages` / `StoreEvent::Messages` 从 app 里删除（`Db::messages` 保留，`asm session` 还在用）。

### 4.2 core 类型与读取

类型放 `crates/core/src/model.rs`（core 的对外类型都在这里），读取放新模块 `crates/core/src/claude/transcript.rs`。

```rust
pub struct Transcript {
    pub entries: Vec<Entry>,
    pub bytes: u64,
}

pub struct Entry {
    pub kind: EntryKind,
    pub ts: Option<i64>,
    /// 1-based prompt number for user entries, 0 otherwise.
    pub turn: u32,
}

pub enum EntryKind {
    User(Marked),
    Assistant(Marked),
    Thinking(Marked),
    Tool(ToolCall),
}

/// Text cut to the per-entry cap, with the search hits inside it.
pub struct Marked {
    pub text: String,
    pub hits: Vec<Range<usize>>,
    /// Characters of the original text, cut or not.
    pub chars: usize,
}

pub struct ToolCall {
    pub name: String,
    /// description `#`, command `$`, then the other fields (`regex::render_tool_use`).
    pub input: Vec<ToolLine>,
    pub result: Option<ToolResult>,
}

pub struct ToolResult {
    pub text: Marked,
    pub bytes: usize,
    pub lines: usize,
    pub is_error: bool,
}
```

规则：

- 先 `classify` 再解析；`Skip` / `Meta` 行不解析。user 记录不能再用 `user_line_may_have_text` 预过滤（tool_result 在这类记录里）。
- 条目按文件顺序；tool_result 按 `tool_use_id` 挂到对应的 tool_use 条目上，找不到对应 tool_use 的（如 compact 之后）单独成一个 `Tool` 条目，`name` 为 `?`。
- `isMeta` 的 user 记录跳过（与现状一致）；空 thinking（只有 signature）跳过；图片 block 不读内容。
- 截断：user / assistant / thinking 每条上限 `ENTRY_CHARS`，tool_result 上限 `RESULT_CHARS`，具体数值在步骤 1 用真实数据定。给了 matcher 且首个命中落在上限之外时，截取窗口移到命中前 `HIT_LEAD_CHARS` 处——即现在 `shown_text` 的行为，整体搬进 core。
- 命中由 core 用 `Matcher` 计算（app 不依赖 `regex`）。工具输入的高亮直接来自 `render_tool_use`，它需要 `&Regex`：给 `Matcher` 加一个 crate 内可见的访问方法。
- shell 命令**不在读取时**调 `format_command`：它每条命令起一个 `shfmt` 进程，112 条就是几百毫秒。步骤 6 在展开卡片时才格式化。
- subagent transcript 与主 transcript 同格式，用同一个 `read`，只是路径不同。

### 4.3 为什么每次重读而不缓存

搜索变化时带新 matcher 重新发请求，线程重新 mmap 并解析。页缓存是热的，70MB 约 100ms，典型文件（中位数远小于 9MB）十几毫秒，且在后台线程。好处是内存里只有截断后的文本，命中落在截断之外的情况由读取时的窗口移动解决，不需要保留全文。

备选：线程缓存最近一个 session 的全文，搜索变化时只重新标记。省掉重读，但工具输出多的 session 要占几十 MB。实测重读不够快时再换。

### 4.4 界面

- `DetailState`：`raw: Vec<MessageRow>` 换成 `Arc<Transcript>` 转换后的 `Conversation`；`scroll: ScrollHandle` 换成 `ListState`；新增 `shown: Vec<usize>`（chip 过滤后的条目序号，做法同表格的 `ix`）。
- `ListState::reset_with_uniform_height(shown.len(), COLLAPSED_ROW_H)` 在数据或 chip 变化时调用；展开 / 折叠调 `remeasure_items(ix..ix + 1)`。
- `park`：`list.scroll_to(ListOffset { item_ix, offset_in_item: px(0.) })`，`item_ix` 是命中条目在 `shown` 里的位置。没有命中时回到顶部。
- 跳到折叠着的工具 / thinking 条目时自动展开它；折叠行上有命中的，在行尾显示命中标记，不展开也能看出来。
- `list` 的渲染回调拿到的是 `&mut App` 而不是 `Context<AppRoot>`：回调里用 `entity.update(cx, |this, cx| this.render_entry(ix, cx))`，现有的 `render_user` / `render_assistant` / `render_thinking` / `render_tool` 签名不用改。
- `Message` 增加 `input_highlights` / `output_highlights`；`render_thinking` 与 `render_tool` 的展开内容改用 `StyledText::with_highlights`。
- `USER #n` 取 `Entry::turn`，`skipped` / `skipped_turns` / `lead_children` 全部删除。
- 删除 `table_hit` 与 `conversation_note` 里关于"工具调用和 thinking 尚未显示"的说明；history-only 的说明保留到步骤 10。
- 读取期间（请求已发、事件未到）显示骨架行（`gpui_kit::component::skeleton::Skeleton`），不显示上一个 session 的内容。

## 5. 步骤

每一步先写行为测试再实现（BDD）。测试名沿用仓库现有的整句风格。

### M4a：数据通路与消息流

**步骤 1：core 读取 transcript**

- 文件：`crates/core/src/model.rs`、`crates/core/src/claude/transcript.rs`（新）、`crates/core/src/claude/parse.rs`（`RawBlock` 加 `id` / `tool_use_id` / `is_error`）、`crates/core/src/claude/mod.rs`、`crates/core/src/search/mod.rs`（`Matcher` 的 crate 内访问方法）、`crates/core/src/bin/asm.rs`（`asm session ID --transcript`，打印条目摘要与读取耗时）、`crates/core/tests/transcript.rs`（新）、`crates/core/tests/fixtures/gen_fixtures.py`
- fixture 需要补：`is_error` 的 tool_result、超过 `RESULT_CHARS` 的 tool_result、没有对应 tool_use 的 tool_result
- 行为测试
  - `a_transcript_reads_as_prompts_replies_thinking_and_tool_calls_in_file_order`
  - `a_tool_result_is_attached_to_the_call_it_answers`
  - `a_tool_result_without_its_call_stands_alone`
  - `injected_prompts_and_empty_thinking_are_left_out`
  - `prompts_are_numbered_from_the_start_of_the_session`
  - `a_long_tool_result_is_cut_and_keeps_its_real_size_and_line_count`
  - `a_failed_tool_call_is_marked`
  - `search_hits_are_marked_in_every_kind_of_entry`
  - `a_hit_past_the_cut_moves_the_cut`（取代 app 里的 `a_hit_deep_inside_a_long_message_moves_the_cut`）
  - `a_subagent_transcript_reads_like_a_session`
  - `a_half_written_last_line_is_ignored`
- 完成标准：`cargo test -p agent-session-core` 全过；`make parity` 仍是 21/21；`asm session <70MB 的 id> --transcript` 在 release 下的读取耗时记入笔记

**步骤 2：store 读取线程与 model 转换**

- 文件：`crates/app/src/store.rs`、`crates/app/src/model.rs`、`crates/app/src/main.rs`
- 行为测试
  - store：`a_transcript_is_served_on_request`（取代 `messages_of_a_session_are_served_on_request…` 的前半段）、`a_transcript_is_served_while_a_scan_is_running`、`a_newer_request_supersedes_the_one_before`、`a_missing_transcript_is_reported_instead_of_crashing`
  - model：`the_conversation_shows_every_kind_of_entry`（取代 `the_conversation_is_the_indexed_texts_without_injected_messages`）、`a_session_with_hundreds_of_messages_is_shown_in_full`（取代 300 条上限的断言）、`a_hit_in_the_400th_message_keeps_its_real_turn_number`（取代 `a_hit_beyond_the_cap_moves_the_window_and_keeps_the_turn_numbers`）、`a_tool_call_shows_its_name_argument_and_result_summary`、`search_hits_are_marked_in_tool_input_and_output`
- 删除：`CONVERSATION_CAP`、`MESSAGE_CHARS`、`WINDOW_LEAD`、`HIT_LEAD_CHARS`、`shown_text`、`ConversationView::{skipped, skipped_turns, total}`、`MessageKind` 上的 `#[allow(dead_code)]`

**步骤 3：虚拟化与命中定位**（依赖 D1）

- 文件：`crates/app/src/detail.rs`
- 行为测试（`ListState` 不需要窗口就能构造，`logical_scroll_top()` 可以断言）
  - `the_view_parks_on_the_first_hit_and_steps_around_the_others`（改写输入类型，断言不变）
  - `a_new_search_marks_the_same_messages_again_and_no_search_marks_nothing`（同上）
  - `parking_scrolls_the_list_to_the_message_with_the_hit`
  - `a_hit_hidden_by_the_chips_leaves_the_scroll_position_alone`
  - `turning_a_chip_off_keeps_the_hit_order_of_what_is_left`
- 必须保住：`n` / `shift-n`（表格有焦点时）、`cmd-g` / `cmd-shift-g`、`▲ 2 / 9 hits ▼`

**步骤 4：工具 / thinking 内的命中**（依赖 D5）

- 文件：`crates/app/src/detail.rs`、`crates/app/src/model.rs`
- 行为测试
  - `a_hit_inside_a_tool_call_counts_and_can_be_stepped_to`
  - `stepping_to_a_hit_in_a_collapsed_entry_opens_it`
  - `collapsing_everything_keeps_the_current_hit_open`
- 删除 `table_hit` 与过渡说明

**步骤 5：验收 M4a**（依赖 D4）

- 文件：`crates/app/src/main.rs`（`ASM_DETAIL_AUTOPLAY`）、`scripts/screenshot-detail.sh`（新）、`Makefile`（`screenshot-detail`）、`scripts/screenshot-clicks.sh`（恢复 thinking / tool 行的点击，校对坐标）
- autoplay 步骤，每步稳定后打印 `autoplay-detail: <step>`：`conversation`（选中 `ASM_AUTOPLAY_SESSION` 指定的 session，缺省取工具调用最多的一行）→ `tool-expanded` → `expand-all` → `regex-tools-hit`（正则 + Tools 范围）→ `next-hit` → `chip-tools-off` → `big-session`（本机最大的 transcript）
- app 在对话首帧上屏后打印一行 `detail: <id> <bytes> read <ms> first frame <ms>`（起点 `select_row`，终点 `window.on_next_frame`）
- 截图归档 `tmp/<date>-m4a-detail/`，逐张用 `Read` 看过
- 提交前：`cargo test --workspace`、`make parity`

### M4b：其余 R6 条目

**步骤 6：工具卡片格式**（R6.4）

- 展开卡片时才对 command 调 `format_command`，经读取线程执行（新请求 `StoreRequest::FormatCommand`），结果回来后替换文本并 `remeasure_items`
- 连续工具调用分组（设计稿的 `groupTools`）**待核对设计稿**：claude-design MCP 在 2026-09-28 连接失败（HTTP 403），需要用户 `/design-login` 后再看；看不到就不做分组并记入 known-issues

**步骤 7：头部与操作按钮**（R6.2、R6.3，依赖 D6）

- 头部增加 token 用量、修改过的文件列表（折叠）
- 按钮：复制 session ID、复制 `claude --resume <id>`（`cx.write_to_clipboard`）、在 Finder 显示 transcript（`cx.reveal_path`）、打开项目目录（`cx.open_with_system`）
- history-only session 没有 transcript：Finder 按钮禁用；匿名 session 没有 id：复制按钮禁用
- 验收时不动用户的剪贴板：按钮的动作抽成返回 `String` 的纯函数并测试它，autoplay 不触发复制

**步骤 8：subagent**（R6.6）

- core：`discover.rs` 增加 `subagents(transcript) -> Vec<SubagentInfo>`（路径 + `meta.json` 的 `agentType` / `description` / `toolUseId`）
- 界面：头部列出 subagent，点击后消息流切换到它的 transcript，再点主 session 返回；对应的 Agent 工具卡片上按 `toolUseId` 显示跳转入口

**步骤 9：Markdown**（R6.4，依赖 D3）

- assistant 正文用 `TextView::markdown`；超长正文先按 `ENTRY_CHARS` 截断再交给它
- 长消息默认折叠 5 行：`max_lines(5)` + `TextViewState::is_clamped()` 决定是否显示"展开"
- 重新跑步骤 5 的计时，Markdown 让首帧超出 200ms 时，视口外的条目先用纯文本占位

**步骤 10：history-only session**（R6.7）

- 读取线程按需读 `history.jsonl`，取该 session 的 prompt 列表，显示为只有 user 条目的消息流，顶部提示 transcript 已被清理

**步骤 11：导出 TXT**（R6.3；2026-10-10 按用户要求改用 XML 标签区分角色）

- core 已实现：`transcript::read_transcript(path)` 返回 `TranscriptExport { text, skipped_lines }`，保留用户输入、最终回复及工具计数；格式和边界测试见 `../notes/2026-10-10-transcript-reader-xml.md`。
- 界面：`cx.prompt_for_new_path` 选 `.txt` 位置后在读取线程调用导出接口并写文件，有 skipped_lines 时提示；不要从当前消息展示窗口生成导出。

### 每段完成后的文档

- `AGENTS.md`"当前状态"、PRD 里程碑表、`../docs/product-prd.md` 状态表与 §4.5
- `../docs/app-architecture.md`：线程表加上 transcript 读取线程，事件表里 `Messages` 换成 `Transcript`，删掉"扫描期间请求要排队"里关于详情的那一段
- 新笔记 `kb/notes/<date>-m4-detail.md`：数据流、截断规则、实测数字、与本方案的差异
- `../known-issues.md`：删掉已解决的条目，登记新发现的
- commit

## 6. 验收

| 项 | 标准 |
|---|---|
| 打开速度 | 见 D4；日志行 `detail: …` 为准 |
| 消息流 | 四种条目都显示；几千条的 session 滚动流畅；chip 计数与条目数一致 |
| 命中 | 正则 + Tools 的命中在详情里高亮并可跳转；`n` / `shift-n` / `cmd-g` / `▲ ▼` 行为与 M3 一致 |
| 扫描期间 | 冷启动索引进行中选中行，对话照常出现 |
| 回归 | `cargo test --workspace` 全过（现为 122 个）；`make parity` 21/21；`make screenshot-search` 七张截图与 M3 一致 |

验收时的硬性规则（来自 M3 的教训）：

- 不用 System Events / osascript / cliclick 发按键，不动剪贴板。交互一律用环境变量驱动的 autoplay，脚本等日志标记再按窗口 ID 截图
- cliclick 鼠标点击只用于无法程序化打开的弹出层
- 截图前 `wake-screen`；截图必须用 `Read` 打开看过
- 手动起的 app 验证完要 kill，`pgrep -fl agent-session-manager` 确认

## 7. 风险

| 风险 | 对策 |
|---|---|
| `list` 的估算高度让滚动条滑块跳动 | 用统一高度提示；仍明显则记入 known-issues，不用 `measure_all` |
| Markdown 首帧布局超出预算 | 步骤 9 放在 M4a 验收之后；超出时视口外条目用纯文本占位 |
| shfmt 拖慢打开 | 只在展开卡片时格式化 |
| 正在写入的 session：mmap 时文件在增长，最后一行不完整 | 丢弃不完整的末行（`TranscriptParser::feed` 的现有做法） |
| 改 `parse.rs` 影响索引与搜索 | `RawBlock` 只加 `Option` 字段；跑 `make parity` |
| 设计稿无法核对 | 见步骤 6；不凭记忆实现设计稿细节 |
| transcript schema 继续变化 | 所有新字段 `Option`，未知 block 类型忽略 |

## 8. 不在 M4 范围内

- 实时更新（对话随 transcript 追加而刷新）：M5 的 R7
- 在终端打开（见 D6）
- subagent 作为表格子行、搜索包含 subagent：R9，M5
- 代码块语法高亮：需要 gpui-kit 的 `tree-sitter-*` feature，编译时间与体积另行评估
