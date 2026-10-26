---
created: 2026-10-26
tags:
  - m4
  - m4a
  - detail-panel
  - conversation
  - turn-cards
  - spec
  - agent-session-manager
---

# M4a：详情面板直接读记录文件，按回合卡显示完整对话

总 plan：[M4 详情面板](2026-10-26-m4-detail-panel.md)。回合、AUTO 卡、compact 分隔线、步骤、刻度条、命中位置这些概念在总 plan 的"共用概念"里定义，这里不重复。对应画稿 1–4。

## PRD

### 背景

详情面板的 Conversation 是过渡实现：只有索引里的 user / assistant 文本，前 300 条，每条截到 4000 字。工具调用和 thinking 看不到，它们里面的搜索命中也看不到；扫描进行中选中一行，对话要等扫描结束才出现。这一期把它换成直接读记录文件、按回合组织的完整对话。

### 目标

选中任何 session，200ms 内看到按回合折叠的完整对话；可以一层层展开到某一步的输入和输出；搜索命中在哪里都能跳过去。

### 功能与交互

#### 1. Conversation 顶栏

原来的 User / Assistant / Tools / Thinking 开关栏换成一行顶栏：

- 左边是概况："9 turns · 214 tool calls"。有 AUTO 卡时写"21 turns · 2 auto · 208 tool calls"。
- 有搜索时，右侧出现"▲ 3 / 9 hits ▼"。
- 最右边是"Expand all"：展开所有回合的步骤列表，之后变成"Collapse all"。

#### 2. 回合卡（折叠状态）

画稿 1。对话区是浅灰底，每个回合一张白色卡片，从上往下：

- **标题行**：状态灯、`TURN n`、开始时间、"19 steps · 6m 30s"。有出错的步骤时接着写红色的"1 error"；被打断、API 出错、没有回复时分别写"interrupted"、"API error"、"no reply"。有搜索命中时右边是"N hits"徽章。时间只写时:分；和上一张卡不在同一天时写成"10-17 21:40"，第一张卡不是今天的也带日期。
- **prompt**：最多显示 4 行，超出时下面有"Show all · 18 lines"，点开显示全文，再点收起。
- **附件**：prompt 里粘贴的图片或文档，在 prompt 下面各显示一个灰色小标签，如"image · 1.2 MB"，不显示图片本身。
- **刻度条和统计行**：刻度条一步一格。统计行写各工具的调用次数，多的在前，然后是过程文字和 thinking 的段数，例如"Read 12 · Bash 11 · Edit 9 · Grep 4 · text 2 · thinking 3"。鼠标停在某一格上时，统计行改写成这一步，例如"14 / 29  Bash  cargo test -p agent-session-manager  error · 46 lines"，移开后恢复。点某一格会展开这个回合的步骤，滚到这一步并展开它。
- **最终回复**：左边标"ASST"和时间，最多显示 6 行，超出时有"Show all · 14 lines"。
- **Show steps ▸**：在卡片底部，点开后变成"Hide steps ▾"。
- 没有步骤的回合，不显示刻度条、统计行和 Show steps。

折叠按原文的行数算：一行很长、显示时自动换成多行，仍算一行。一整段没有换行的长文最多显示约 900 字，超出的部分同样用"Show all"展开。

#### 3. 展开步骤

画稿 2。点"Show steps ▸"后，最终回复的上方按顺序列出每一步：

- **工具调用**：一行，依次是时间、▶、工具名（紫色）、参数摘要、结果摘要。
  - 参数摘要取这个工具最有用的字段：Bash 取命令；Read / Edit / Write 取文件路径，项目目录下的写相对路径，家目录写成 `~`；Grep 取搜索词和路径；WebFetch 取网址；Task 取任务描述；其余工具取第一个文字字段。
  - 结果摘要写成"212 lines"或"11 lines · 640 B"；Edit / Write 成功时写"ok"；出错时写红色的"error · 46 lines"。
- **展开工具行**：下面出现 input 和 output 两个等宽框。input 里，description 以 `#` 开头，命令以 `$` 开头，其余字段写成 `key: value`。output 最多显示 12 行，后面写"+ 40 more lines · 4.1 KB"；出错时 output 的文字是红色。
- **thinking**：一行，写斜体灰色的"thinking"、开头一段和长度（"1.2k chars"），点开显示全文。
- **过程文字**：不折成一行，直接显示成一段 ASST 文字，和最终回复一样最多显示 6 行。
- 展开状态在选中期间保留。换一个 session，所有内容回到折叠状态。

#### 4. 特殊回合

画稿 4。

- **被打断**：用户按 Esc 打断时，卡片底部（最终回复的位置）显示一行灰色的"[Request interrupted by user]"和时间，红灯，标题行写"interrupted"。打断之后用户接着输入的内容是下一个回合。
- **API 出错**：卡片底部显示红色的 API 错误文字，最多 2 行，红灯，标题行写"API error"。同一回合里之后重试成功、有了正常回复，就按正常结束显示。
- **没有回复**：灰灯，标题行写"no reply"。session 还在进行时，最后一个回合就是这样。
- **AUTO 卡**：标题写 `AUTO` 和来源标签（如 task-notification、auto-continuation、peer），prompt 是那条消息的内容，其余部分和回合卡一样。第一条真实输入之前如果已经有 Agent 的活动（例如 SDK 发起的 session），这些步骤放在一张来源标签为"session start"的 AUTO 卡里。
- **compact 分隔线**：一条横线，写"context compacted · 10-17 22:05"和"summary · 9.8k chars ▸"，点开显示压缩摘要的全文。

#### 5. 搜索命中

画稿 3。

- 选中一行时如果有搜索，对话直接停在第一个命中位置：命中所在的回合自动展开步骤列表；命中在某一步里时，这一步也自动展开；命中在被折叠的 prompt、回复或工具输出的后部时，这一段自动显示全文。
- 命中的文字是黄色底，当前命中是橙色底。
- 卡片标题行的"N hits"是这张卡里的命中位置数。刻度条上含命中的格子带黄圈，当前命中的那一格是橙圈。折叠着的步骤行如果含命中，行尾显示命中数。
- "▲ ▼"按钮、`n` / `⇧N`（表格有焦点时）、`⌘G` / `⇧⌘G` 在所有命中位置之间循环；跳到哪里，就按上面的规则展开到哪里。
- 很长的文字只保留一部分：prompt、回复、过程文字、thinking 各保留前 2 万字，工具输出保留前 3000 字。第一个命中在保留范围之后时，保留的那一段移到命中附近，开头用"…"表示前面有省略。
- 改了搜索词：重新标记，停在新搜索的第一个命中，已经展开的回合保持展开。清空搜索：高亮消失，回到顶部。

#### 6. 读取中、出错和空状态

- 读取期间显示几行骨架占位，不显示上一个 session 的内容。
- 读取失败（文件刚被删除、没有读权限）：对话区显示一行错误说明。
- 记录文件已被清理的 History session：仍显示现在的说明文字；prompt 列表在 M4b 做。
- 一条消息都没有的 session：显示"No messages in this session"。
- 正在写入的 session：最后一行还没写完时忽略这一行，显示到上一行为止。

#### 7. 扫描期间

冷启动或重建索引时选中一行，对话照常出现，不用等扫描结束。

#### 8. 打开速度

从选中一行到对话出现：本机最大的记录文件（70MB）不超过 200ms，以 release 构建为准。

### 不做的事（本期）

- 标题行的操作按钮、右键菜单的扩展、快捷键复制、导出 TXT：M4b
- History session 的 prompt 列表：M4b
- 代码块的等宽框：M4b。本期回复按纯文本显示，``` 原样出现
- `[` / `]` 按回合跳转：M4b
- Metadata tab 不变

### 验收标准

1. 顶栏写出回合数、AUTO 数和工具调用数，与卡片一致；没有 User / Assistant / Tools / Thinking 开关
2. 每个回合一张卡，编号从 1 起连续；AUTO 卡不占编号
3. 折叠的卡片显示状态灯、步数、用时、出错数、prompt（最多 4 行）、刻度条、统计行和最终回复（最多 6 行）；超出的部分能用 Show all 展开
4. 鼠标停在某一格刻度上，统计行变成这一步的说明；点这一格会展开并滚到这一步
5. Show steps 展开后每一步一行；工具行能展开看 input / output，thinking 能展开看全文，过程文字直接显示
6. 被打断、API 出错、没有回复、AUTO、compact 都按画稿 4 显示
7. 粘贴了图片的 prompt 显示图片占位和大小
8. 正则搜索命中工具输出时，详情停在那一步，这一步已展开，命中高亮；`n` / `⇧N` / `⌘G` / `⇧⌘G` / ▲▼ 能在全部命中之间循环
9. 消息超过 300 条的 session 能看到最后一个回合
10. 冷启动扫描进行中选中一行，对话立即出现
11. 70MB 的 session 从选中到对话出现不超过 200ms（release）
12. 读取期间显示骨架；读取失败时显示说明

## Tech Design

### 现状

**app**

- `crates/app/src/detail.rs`：`DetailState` 持有 `raw: Vec<MessageRow>`、`view: ConversationView`、`hits`、`current_hit`、`scroll: ScrollHandle`、`chips: Chips`、`expanded`、`force_all`。`park()` 用 `scroll_to_top_of_item(child + lead_children())`，前提是每条消息都是滚动区的直接子元素。渲染函数有 `render_chip_bar`、`render_hit_nav`、`render_user` / `render_assistant` / `render_thinking` / `render_tool`、`collapsed_row`、`table_hit`、`conversation_note`。标题行的 ★ 是 `Button::new("favorite-session")`，这期不动。文件里还有两处直接写的色值 `rgb(0xc4c4c8)`、`rgb(0xf2f2f4)`，违反"色值只在 `style.rs`"的规则。
- `crates/app/src/model.rs`（约 590–720 行）：`MessageKind`、`Message`、`type Conversation = Vec<Arc<Message>>`、`CONVERSATION_CAP` / `MESSAGE_CHARS` / `WINDOW_LEAD` / `HIT_LEAD_CHARS`、`ConversationView`、`shown_text`、`conversation_from`，以及测试 `the_conversation_is_the_indexed_texts_without_injected_messages`、`search_hits_are_marked_in_the_conversation`、`a_hit_beyond_the_cap_moves_the_window_and_keeps_the_turn_numbers`、`a_hit_deep_inside_a_long_message_moves_the_cut`。
- `crates/app/src/store.rs`：`StoreRequest::Messages(String)` → `Worker::messages` → `Db::messages` → `StoreEvent::Messages`，和扫描排在同一条线程上；测试 `messages_of_a_session_are_served_on_request_and_a_rebuild_streams_everything_again`。
- `crates/app/src/main.rs`：`select_row`（约 396 行）在换 session 时调 `detail.clear()` 并发 `Messages` 请求；`on_store_events`（约 334 行）把 `Messages` 交给 `detail.set_messages`。`crates/app/src/toolbar.rs:200` 的 `run_search` 每次都调 `detail.set_matcher`，而 `run_search` 在过滤变化、扫描后整表重读时也会跑。
- `crates/app/src/keyboard_tests.rs`：`enter_searches_at_once_and_n_steps_through_the_hits_while_the_table_has_the_focus` 断言 fixture 里搜 "parser" 得到 `(1, 3)`；`matches_header_sort_keeps_the_selected_session_and_detail_hit` 断言排序不会重载详情。
- `scripts/screenshot-clicks.sh:51` 点击 Assistant chip（第 11 张截图），53 行的注释说 M4 后要补回 thinking / tool 行的点击。

**core**

- `crates/core/src/claude/parse.rs`：`RawBlock` 只有 `kind` / `text` / `thinking` / `name` / `input` / `content`，没有 `id`、`tool_use_id`、`is_error`、`source`。`classify` 把 `system` 记录归为 `Skip`。`parse_content`、`user_text`、`map_file` 可复用。
- `crates/core/src/transcript.rs`（TXT 导出）：模块内的 `Record` 有 `kind`、`uuid`、`subtype`、`is_meta`、`is_sidechain`、`is_compact_summary`、`is_aborted`、`origin`、`message { id, stop_reason, content }`。`human_text` 判断真实输入并取文本（会把图片写成 `[Image omitted]`）。`Turn::assistant` 是最终回复的规则：带 tool_use 的消息里的文字是过程文字；同一个 `message.id` 拆成多条记录时合并；更早的纯文字消息被后来的替换。
- `crates/core/src/search/mod.rs`：`Matcher { re }`，`ranges(text)` 返回全部命中区间，`re` 是私有的。
- `crates/core/src/search/regex.rs`：`ToolLine` / `ToolLineKind` / `Highlighted` 是 `pub`；`render_tool_use` 会调 `format_command`（每条长命令起一个 `shfmt` 进程），不能用在读取上；`stringify` 是私有的。
- fixture：`crates/core/tests/fixtures/gen_fixtures.py` 生成 `fixtures/claude/`；`reader.jsonl` 放在发现目录之外，"so existing index/parity fixtures stay unchanged"。

**真实数据（2026-10-26 在本机 `~/.claude/projects` 上核对）**

- 打断标记是一条 user 记录：文字为 `[Request interrupted by user]` 或 `[Request interrupted by user for tool use]`（偶尔带结尾换行），带 `interruptedMessageId`，没有 `origin` 和 `isMeta`。85 个文件里有。它能通过导出的 `human_text`，所以 TXT 导出会把它当成一条用户输入（已登记到 known-issues，M4b 修）。
- API 错误是一条 assistant 记录：`isApiErrorMessage: true`，`message.model` 为 `<synthetic>`，正文以 "API Error:" 开头。722 个文件里有。
- `isAbortedMidStream: true` 的 assistant 记录：2 个文件。
- `system` / `compact_boundary`：90 个文件；压缩摘要是紧跟着的一条 `isCompactSummary: true` 的 user 记录。
- `origin.kind` 按行计：human 2073、task-notification 960、auto-continuation 60、peer 15、coordinator 11。

### 方案

core 新增 `claude/conversation.rs`，把一份记录文件读成 `Conversation`：按回合切分，每个回合是 prompt、按顺序排的步骤、最终回复和结束状态；给了 `Matcher` 时顺便标出命中，长文本按上限截断（截断窗口跟着第一个命中移动）。store 里新开一条读取线程，按 `seq` 取代旧请求。app 新增 `conversation.rs`，把 `Conversation` 转成预先格式化好的卡片数据和一张"行表"；`detail.rs` 用 `list` + `ListState` 渲染行表。搜索词变化时带新的 `Matcher` 重读文件，保留已展开的状态。

关键决策：

1. **和 TXT 导出共用记录结构和"真实输入"规则**。conversation 需要的字段（`origin`、`isCompactSummary`、`isApiErrorMessage`、`isAbortedMidStream`、`interruptedMessageId`、`subtype`、`durationMs`、`timestamp`）对索引没有用，加进 `parse.rs` 的 `RawRecord` 会让每次索引多解码。导出模块的 `Record` 已经有一大半，把它改成 `pub(crate)` 并补齐字段，两边共用；`human_text` 拆成"是否真实输入"和"取文本"两步，导出的行为不变。否决：conversation 再定义一份 `Record`（两份规则迟早不一致）。
2. **最终回复沿用导出的规则**。步骤按顺序收集，过程文字步骤记下它的 `message.id` 和"这条消息是否带工具调用"。回合结束时，从末尾取出属于最后一条纯文字消息的那几段文字（之后没有工具调用和 thinking），合并成最终回复；其余留作过程文字步骤。
3. **打断和 API 错误结束当前回合，不开新回合**。判断依据见上面的真实数据。一个回合的结束状态取最后发生的那件事：最后是正常回复就是正常结束（重试成功的情况）。
4. **列表的一项是卡片的一段**（总 plan D12）。行表是 `Vec<Row>`：

   ```rust
   pub enum Row {
       Head(usize),        // 标题行、prompt、附件、刻度条、统计行
       Step(usize, usize), // 回合展开时才有
       Tail(usize),        // 最终回复 / 打断 / API 错误，加 Show steps 开关
       Compact(usize),     // compact 分隔线
   }
   ```

   展开或收起一个回合时，用 `ListState::splice` 只替换这张卡的行，不影响其余行的测量和滚动位置。卡片外观由各行拼成：Head 画上边框和上圆角，Tail 画下边框和下圆角，中间的行只画左右边框。M4b 会在这上面加 History 卡（只有 Head 和空的 Tail）和按 Head 跳转。否决：一张卡一项（展开 200 步的卡每帧都要整张布局，命中也只能定位到卡）。
5. **命中按"命中位置"记**：`HitPlace { item, part }`，`part` 是 prompt、某一步、最终回复或压缩摘要。core 读的时候算好，app 只保存位置序列和当前序号。跳到一个位置：所在回合展开 → 所在步骤展开 → 所在段落取消折叠 → 重建这张卡的行 → `list.scroll_to(ListOffset { item_ix: 行号, offset_in_item: px(0.) })`。
6. **只有搜索词变了才重读**。给 `Matcher` 加 `pattern()`；`run_search` 里对比当前对话读取时用的 pattern，相同就不发请求。
7. **截断在 core，折叠在 app**。截断决定内存里留多少（core 常量）：`ENTRY_CHARS = 20_000`（prompt、回复、过程文字、thinking、压缩摘要）、`RESULT_CHARS = 3_000`（工具输出）、`INPUT_FIELD_CHARS = 2_000`（工具输入的每个字段）、`HIT_LEAD_CHARS = 200`（截断窗口移到命中前多少字，沿用现值）。折叠决定先显示多少（app 常量）：prompt 4 行、回复和过程文字 6 行、工具输出 12 行，另有 `FOLD_CHARS = 900` 防止一整段不换行的长文。步骤 1 用真实数据统计超过上限的比例，数值可以调，改了要同步 PRD 第 5 节。
8. **工具输入在 core 自己排**，不用 `render_tool_use`：顺序相同（description `#`、command `$`、其余 `key: value`），但不调 `format_command`（`shfmt` 每条命令起一个进程，几百条就是几百毫秒），每个字段按 `INPUT_FIELD_CHARS` 截断。`regex::stringify` 改成 `pub(crate)` 复用。参数摘要（折叠行里那一段）的取字段规则也在 core：`tool_arg(name, input)`；路径改成相对路径或 `~` 在 app 做（只有 app 知道项目目录）。
9. **界面上的字符串全部在转换时格式化**：时间、用时、统计行、结果摘要、"Show all · N lines"都在 app 的 `conversation.rs` 里一次算好，渲染只取字符串（AGENTS 的 `render_td` 规则同样适用于这里）。

### 数据变动

不动索引 schema，不提升 `SCHEMA_VERSION`。

**core `model.rs` 新增类型**（AGENTS 规定对外类型放这里；都要 `Debug`，因为 `StoreEvent` 是 `Debug`）：

```rust
pub struct Conversation {
    pub items: Vec<ConvItem>,       // 文件顺序
    pub bytes: u64,                 // 读取时的文件大小
    pub skipped_lines: usize,       // 不是合法 JSON 的行；没写完的最后一行不算
}

pub enum ConvItem { Turn(Turn), Compact(Compaction) }

pub struct Turn {
    pub number: Option<u32>,        // 真实输入的回合从 1 编号；AUTO 段为 None
    pub origin: Option<String>,     // AUTO 段的 origin.kind，或 "session start"
    pub ts: Option<i64>,
    pub prompt: Marked,             // clean_prompt 之后的文字，不含附件占位
    pub attachments: Vec<Attachment>,
    pub steps: Vec<Step>,
    pub reply: Option<Reply>,
    pub end: TurnEnd,
    pub duration_ms: Option<i64>,   // turn_duration 记录的和；没有时 app 用 last_ts - ts
    pub last_ts: Option<i64>,
}

pub struct Attachment { pub kind: String, pub bytes: u64 }   // image / document；base64 长度 × 3 / 4
pub struct Step { pub ts: Option<i64>, pub kind: StepKind }
pub enum StepKind { Text(Marked), Thinking(Marked), Tool(ToolCall) }

pub struct ToolCall {
    pub name: String,
    pub arg: String,                // 参数摘要，单行，最多 300 字
    pub input: Vec<ToolLine>,       // search::regex::ToolLine，带命中区间
    pub result: Option<ToolResult>, // 没等到结果（被打断、还在跑）时为 None
}

pub struct ToolResult { pub output: Marked, pub bytes: usize, pub lines: usize, pub is_error: bool }
pub struct Reply { pub ts: Option<i64>, pub text: Marked }
pub enum TurnEnd { Completed, Interrupted { ts: Option<i64> }, ApiError { ts: Option<i64>, text: String }, NoReply }
pub struct Compaction { pub ts: Option<i64>, pub summary: Option<Marked> }

/// 截到上限的文字和其中的命中。
pub struct Marked {
    pub text: String,
    pub hits: Vec<Range<usize>>,
    pub chars: usize,               // 原文的字数和行数，截没截都是原文的
    pub lines: usize,
    pub moved: bool,                // 截断窗口移到了命中附近，text 以 … 开头
}

pub enum Part { Prompt, Step(usize), Reply, Summary }
pub struct HitPlace { pub item: usize, pub part: Part }
```

`Conversation::hit_places(&self) -> Vec<HitPlace>` 按文件顺序列出每个含命中的位置。

**core `parse.rs`**：`RawBlock` 加 `id`、`tool_use_id`（`Option<Cow<str>>`）、`is_error`（`Option<bool>`）、`source`（`Option<RawSource>`，只取 `data: Option<&RawValue>` 算长度，不解码 base64）。都是 `Option`，索引和搜索不受影响。

**core `transcript.rs`**：`Record` 改为 `pub(crate)`，加 `timestamp`、`is_api_error`（`isApiErrorMessage`）、`interrupted_message_id`、`duration_ms`（`durationMs`）。

**app**：`StoreRequest::Messages` / `StoreEvent::Messages` 删除，新增 `StoreEvent::Conversation`（见逻辑变动）。`Db::messages` 保留，`asm session` 还在用。

### 逻辑变动

#### core

- **`claude/conversation.rs`（新）**：`pub fn read_conversation(path: &Path, matcher: Option<&Matcher>) -> Result<Conversation>`。逐行处理的规则，按顺序：
  1. `map_file` 映射整个文件，按 `\n` 切行；空行跳过。行解析失败计入 `skipped_lines`，但最后一行没有结尾换行时直接忽略（正在写入）。
  2. `isSidechain` 且不是 `subagents/` 下的文件 → 跳过（同导出）。`uuid` 重复 → 跳过。
  3. `system`：`compact_boundary` → 结束当前回合，追加 `ConvItem::Compact`；`turn_duration` → 加到当前回合的 `duration_ms`；其余忽略。
  4. `user`：
     - `isCompactSummary` → 填进最近的那个 `Compaction.summary`；前面没有 `compact_boundary`（旧文件）就新建一个。
     - 含 `tool_result` block → 按 `tool_use_id` 找到等结果的工具步骤（`HashMap<String, (item, step)>`），填 `result`；找不到的作为一个 `name` 为 `?` 的工具步骤加到当前回合。
     - 打断标记（带 `interruptedMessageId`，或文字 trim 后以 `[Request interrupted by user` 开头）→ 当前回合 `end = Interrupted`。要在真实输入判断之前检查。
     - 真实输入（`is_human_input`，即拆出来的导出规则）→ 结束当前回合，开一个编号 +1 的回合；prompt 是 `clean_prompt(user_text)`，`image` / `document` block 记为附件。
     - `origin.kind` 存在且不是 `human`、有文字 → 结束当前回合，开一个 AUTO 段，`origin` 为这个 kind。
     - 其余（`isMeta`、以 `[Image: source:` 开头的图片载体）→ 跳过。
  5. `assistant`：
     - `isApiErrorMessage` → 当前回合 `end = ApiError`，文字取正文。
     - `isAbortedMidStream` → `end = Interrupted`。
     - 否则按 block 顺序：非空 `thinking` → Thinking 步骤；`tool_use` → Tool 步骤，按 `id` 登记等结果；`text` → Text 步骤，记下 `message.id` 和"这条消息是否带工具调用（或 `stop_reason == tool_use`）"。正常的 block 出现在 ApiError / Interrupted 之后时，清掉这个结束状态。
     - 第一条真实输入之前出现的 assistant 内容，放进一个 `origin = "session start"` 的 AUTO 段。
  6. 回合结束（遇到新回合、compact 或文件末尾）：按决策 2 取出最终回复；`end` 没被设置时，有回复是 `Completed`，否则 `NoReply`。
  7. 文字统一经 `Marked::new(text, cap, matcher)`：先按 `cap` 截断；第一个命中落在截断位置之后时，窗口移到命中前 `HIT_LEAD_CHARS` 字（就是现在 app 里 `shown_text` 的规则，整体搬过来），然后在截断后的文字上算命中区间。
  8. 工具输入经 `tool_input_lines(&Map, matcher)`（决策 8）。`tool_arg(name, &Map)`：Bash 取 `command` 的第一行；Read / Edit / Write / NotebookEdit 取 `file_path`；Grep 取 `pattern` 加 `path`；Glob 取 `pattern`；WebFetch 取 `url`；WebSearch 取 `query`；Task / Agent 取 `description`；Skill 取 `skill`；其余取第一个字符串字段。
- **`claude/mod.rs`**：`pub mod conversation;`
- **`transcript.rs`**：`Record` 共享（见数据变动）；`human_text` 拆成 `pub(crate) fn is_human_input(record, content) -> bool` 和原来的取文本部分，导出的输出逐字不变。
- **`search/mod.rs`**：`Matcher::pattern(&self) -> &str`；给 conversation 用的 `pub(crate) fn re(&self) -> &Regex`（如果 `ranges` 不够用）。
- **`search/regex.rs`**：`stringify` 改为 `pub(crate)`。
- **`bin/asm.rs`**：`asm session ID --conversation [--match REGEX]`：每个回合打印一行（编号或 AUTO、步数、结束状态、命中数），最后打印合计与读取耗时。用来在真实数据上核对规则、在 release 下量读取时间。默认输出不变，`make parity` 不受影响。

#### app

- **`store.rs`**：
  - `Store::spawn` 另起一条 `asm-reader` 线程，不持有 `Db`。`Store` 加 `reads: mpsc::Sender<ReadRequest>` 和 `latest: Arc<AtomicU64>`。
  - `pub fn read_conversation(&self, session_id: String, path: PathBuf, matcher: Option<Matcher>) -> u64`：先把 `latest` 加一作为 `seq`，再发请求，返回 `seq`（照 `Searcher::search`）。
  - 读取线程收到请求后，先用 `try_recv` 把队列里更新的请求取完、只留最后一个；开始读之前再看一眼 `latest`，过期就丢掉。读完发 `StoreEvent::Conversation { seq, session_id, conversation: Result<Arc<Conversation>, String>, elapsed }`。
  - 删除 `StoreRequest::Messages`、`StoreEvent::Messages`、`Worker::messages`。
- **`conversation.rs`（新，app 端的转换）**：
  - `ConversationView`：顶栏概况字符串、`cards: Vec<Card>`、`hits: Vec<HitPlace>`、读取时用的 `pattern: Option<String>`。`Card` 是 `Turn(TurnCard)` 或 `Compact(CompactCard)`。`TurnCard` 里全是预先格式化好的字段：标签（`TURN 3` / `AUTO`）、来源标签、时间、状态（ok / failed / off）、步数与用时、出错数、结束说明、prompt、附件标签、刻度（每格的种类、是否出错、是否含命中）、统计行、每一步的显示数据、结尾（回复 / 打断 / API 错误）、命中数。
  - `pub fn build(conv: &Conversation, project_path: &str, today: NaiveDate) -> ConversationView`。
  - `fold(text, max_lines, max_chars) -> Option<usize>`：需要折叠时返回折叠处的字节位置；折叠处之后有命中时返回 `None`（自动显示全文）。
  - 工具参数摘要里的路径：在项目目录下改成相对路径，在家目录下改成 `~/…`，太长时中间用 `…` 省略。
  - 结果摘要：出错 `error · N lines`；Edit / Write / MultiEdit / NotebookEdit 成功 `ok`；其余 `N lines`，输出不小于 512 B 时加 `· 大小`；没有结果时为空。
  - 时间：和上一张卡同一天写 `HH:MM`，否则 `MM-DD HH:MM`；第一张卡不是 `today` 时带日期。用时写成 `45s`、`1m 02s`、`6m 30s`、`1h 12m`（新写一个函数；Metadata 用的 `format_duration` 只到分钟，不合用），没有 `duration_ms` 时用 `last_ts - ts`。
  - `pub fn rows(view: &ConversationView, open: &HashSet<usize>) -> Vec<Row>`（决策 4）。
- **`detail.rs`**：
  - `DetailState` 改为：`view: Option<Arc<ConversationView>>`、`status`（loading / failed(message) / ready）、`rows: Vec<Row>`、`list: ListState`、`open_turns: HashSet<usize>`、`open_steps: HashSet<(usize, usize)>`、`unfolded: HashSet<(usize, Part)>`、`hover_tick: Option<(usize, usize)>`、`hits`、`current_hit`。
  - 方法：`begin_loading()`、`set_conversation(view, keep_open: bool)`（换 session 时 `false`，同一 session 换搜索时 `true`）、`fail(message)`、`clear()`、`toggle_turn` / `toggle_step` / `toggle_fold`、`expand_all(open: bool)`（收起全部时保留当前命中所在的回合和步骤）、`step_hit(forward)`、`hit_position()`、`park()`。`hit_position()` 的签名不变，键盘测试在用。
  - 删除 `Chips`、`render_chip_bar`、`table_hit`、`conversation_note` 里的过渡说明（History 的那句保留到 M4b）、`render_user` / `render_assistant` 和旧的 `render_thinking` / `render_tool`。`render_hit_nav` 保留，放进新顶栏。
- **`turn_cards.rs`（新，`impl AppRoot` 的渲染部分）**：`render_conversation`（顶栏 + `list` + 滚动条）、`render_row(ix)`、`render_head`、`render_step`、`render_tail`、`render_compact`。要点：
  - `list(self.detail.list.clone(), move |ix, window, cx| view.update(cx, |this, cx| this.render_row(ix, window, cx)))`，`view` 是 `cx.entity()`；`list` 的回调拿到的是 `&mut App`，不是 `Context<AppRoot>`。
  - 滚动条：`.vertical_scrollbar(&self.detail.list)`（`gpui-base` 已实现 `ScrollbarHandle for ListState`）。
  - 刻度一格画成 4×12 的竖杠或 5px 的圆点，外面包一个 8px 宽的透明区域接 `on_hover` 和 `on_click`，否则鼠标很难停准。
  - 命中高亮用 `StyledText::with_highlights`，和现在一样。
  - 读取中用 `gpui_kit::component::skeleton::Skeleton` 画 3–4 张卡的轮廓。
- **`main.rs`**：
  - `AppRoot` 加 `conversation_seq: u64` 和 `detail_opened: Option<(SharedString, Instant)>`（计时用）。
  - `select_row` 换 session 时：`detail.begin_loading()`；`row.transcript_path` 存在就 `conversation_seq = store.read_conversation(...)` 并记下起始时间；否则（History）直接显示说明。
  - `on_store_events` 处理 `StoreEvent::Conversation`：`seq` 和选中的 session 都对得上才用。成功时 `detail.set_conversation(conversation::build(...), keep_open)`；失败时 `detail.fail(message)`。首次显示这个 session 时，用 `window.on_next_frame` 打印一行 `detail: <短 id> <bytes> read <ms> first frame <ms>`（起点是 `select_row`）。
- **`toolbar.rs::run_search`**：`self.detail.set_matcher(...)` 换成：pattern 和当前对话读取时的不同，就带新 `Matcher` 重发读取请求（`keep_open = true`，旧内容留在屏幕上直到新的到达）。
- **`style.rs`**：新增 `turn_ok`（#16a34a）、`turn_err`（#dc2626）、`turn_off`（#c4c4c8）、`tick`（#b4b4bb）、`conversation_bg`（#f3f3f5）、`card_border`（#e3e3e6）、`step_border`（#f0f0f2）；把 `detail.rs` 里直接写的两处色值搬过来。命中的黄 / 橙沿用 `hit_highlight` / `hit_current_highlight`，刻度上的圈用同样的两种颜色。其余尺寸和色值以画稿源码为准：用 Artifact 工具读 `https://claude.ai/artifact/M31PYmLyK2m5n8Bizyn9fz` 的 `project/Main.dc.html`、`project/Expanded.dc.html`、`project/Search.dc.html`、`project/Special.dc.html`（`action: read` + `path`）。
- **`main.rs` 的 autoplay**：`ASM_DETAIL_AUTOPLAY=1` 时依次执行以下步骤，每步稳定后打印 `autoplay-detail: <step>` 并停留供截图：
  1. `conversation`：选中 `ASM_AUTOPLAY_SESSION` 指定的 session，缺省取工具调用最多的一行
  2. `steps-open`：展开步骤最多的那个回合
  3. `step-open`：展开其中第一个出错的工具步骤，没有就展开第一个工具步骤
  4. `expand-all`
  5. `search-hit`：设正则查询 `ASM_AUTOPLAY_REGEX`（缺省用能命中工具输出的 `cargo (test|build)`）
  6. `next-hit`
  7. `special`：选中 `ASM_AUTOPLAY_SPECIAL` 指定的 session（有 compact、AUTO、打断的那种；缺省跳过这一步）
  8. `big-session`：选中 `5f6eea99`（不存在就跳过）
- **脚本**：新增 `scripts/screenshot-detail.sh` 和 `make screenshot-detail`，照 `screenshot-search.sh` 的写法等日志标记再按窗口 ID 截图。`scripts/screenshot-clicks.sh` 的第 11 张（Assistant chip）改为点第一张卡的"Show steps"，并补一张点开工具行的截图；改完校对坐标。

### 实施步骤

每一步先写行为测试再实现（BDD）。测试名沿用仓库的整句风格。

- [x] **步骤 1：core 读取对话**
  - fixture：在 `gen_fixtures.py` 里加 `conversation_rows`，写到 `fixtures/conversation.jsonl`（放在发现目录之外，和 `reader.jsonl` 一样，不影响索引和 parity 的 fixture）。要覆盖：两个真实输入的回合；thinking（含只有 signature 的空 thinking）；字符串和数组两种 tool_result；`is_error` 的 tool_result；找不到调用的 tool_result；超过 `RESULT_CHARS` 的输出；过程文字之后、同一 `message.id` 分两条记录写的最终回复；打断标记；API 错误；task-notification；`compact_boundary` 加 `isCompactSummary`；带图片的 prompt；`isMeta`；重复的 uuid；`turn_duration`；最后一行没写完。用 `uv run` 重跑脚本并提交生成结果。
  - 测试（新文件 `crates/core/tests/conversation.rs`）：
    - `turns_start_at_human_inputs_and_are_numbered_from_one`
    - `steps_keep_file_order_and_the_last_text_only_message_is_the_reply`
    - `a_reply_written_as_two_records_of_one_message_is_joined`
    - `a_tool_result_is_attached_to_the_call_it_answers`
    - `a_tool_result_without_its_call_becomes_a_step_of_its_own`
    - `a_failed_tool_call_is_marked`
    - `an_interruption_ends_the_turn_without_starting_a_new_one`
    - `an_api_error_ends_the_turn_and_a_later_reply_clears_it`
    - `a_turn_without_a_reply_ends_as_no_reply`
    - `notifications_form_unnumbered_auto_segments_with_their_origin`
    - `activity_before_the_first_input_forms_a_session_start_segment`
    - `a_compaction_is_a_divider_carrying_its_summary`
    - `images_in_a_prompt_become_attachments_with_their_size`
    - `injected_messages_duplicates_and_empty_thinking_are_left_out`
    - `turn_durations_add_up`
    - `a_half_written_last_line_is_ignored`
    - `a_long_tool_result_is_cut_and_keeps_its_real_size_and_line_count`
    - `tool_input_lists_description_command_then_the_other_fields`
    - `the_tool_argument_is_the_most_telling_field`
    - `search_hits_are_marked_in_prompts_steps_replies_and_summaries`
    - `a_hit_past_the_cut_moves_the_cut`（取代 app 里的 `a_hit_deep_inside_a_long_message_moves_the_cut`）
    - `hit_places_list_every_part_with_a_hit_in_file_order`
    - `a_subagent_transcript_reads_like_a_session`
  - 完成标准：`cargo test -p agent-session-core` 全过，`transcript_reader` 的测试不改也能过；`make parity` 21/21；在真实数据上跑 `asm session <id> --conversation`，核对打断、API 错误、AUTO、compact 的识别数和上面的真实数据对得上；release 下 `5f6eea99` 的读取耗时和截断上限的统计记进 session 总结。
- [x] **步骤 2：store 读取线程**
  - 测试（`store.rs`）：
    - `a_conversation_is_served_on_request`（取代 `messages_of_a_session_are_served_on_request…` 的前半段；后半段改名为 `a_rebuild_streams_everything_again` 单独保留）
    - `a_conversation_is_served_before_the_first_scan_settles`
    - `a_newer_request_supersedes_the_one_before`：连发两个请求，收到的最后一个 `Conversation` 事件带最新的 `seq`，旧 `seq` 的事件（如果有）都在它之前
    - `a_missing_transcript_is_reported_instead_of_crashing`
- [x] **步骤 3：app 转换（`conversation.rs`）**
  - 测试：
    - `the_summary_counts_turns_auto_segments_and_tool_calls`
    - `a_card_shows_steps_duration_errors_and_how_the_turn_ended`
    - `stats_list_tools_by_count_then_text_and_thinking`
    - `ticks_follow_the_steps_and_mark_errors_and_hits`
    - `a_tool_step_summarizes_its_argument_and_result`（Bash 命令、项目内相对路径、`~`、Edit 成功为 ok、出错的行数）
    - `folding_counts_source_lines_and_caps_one_long_line`
    - `a_hit_inside_a_fold_opens_it`
    - `times_carry_the_date_when_the_day_changes`
    - `a_closed_turn_is_a_head_and_a_tail_and_an_open_turn_lists_its_steps_between`
  - 删除 app `model.rs` 里的 `MessageKind`、`Message`、`Conversation`、`ConversationView`、`conversation_from`、`shown_text` 和四个常量，以及"现状"里列出的四个测试。
- [x] **步骤 4：详情状态与渲染（`detail.rs`、`turn_cards.rs`、`style.rs`）**
  - 测试（`DetailState` 是纯状态；`ListState` 不需要窗口就能构造，`logical_scroll_top()` 可以断言）：
    - `the_view_parks_on_the_first_hit_and_steps_around_the_others`（改写输入，断言不变）
    - `a_new_search_keeps_open_turns_and_parks_on_its_first_hit`
    - `no_search_marks_nothing_and_parks_at_the_top`
    - `stepping_to_a_hit_in_a_closed_turn_opens_the_turn_and_the_step`
    - `parking_scrolls_the_list_to_the_row_of_the_hit`
    - `opening_a_turn_splices_its_steps_in_and_leaves_the_rows_above_alone`
    - `expand_all_opens_every_turn_and_collapse_all_keeps_the_current_hit_open`
    - `a_new_session_starts_folded`
  - 界面：照画稿 1–4 实现顶栏、卡片各行、刻度的悬停和点击、骨架、错误和空状态。
- [x] **步骤 5：接线（`main.rs`、`toolbar.rs`）**
  - 测试（`keyboard_tests.rs`）：
    - `enter_searches_at_once_and_n_steps_through_the_hits_while_the_table_has_the_focus`：命中单位变成命中位置后，fixture 里 "parser" 的位置数会变；按新规则在 fixture 上数出来，改掉期望值，循环跳转的断言保持
    - `matches_header_sort_keeps_the_selected_session_and_detail_hit`：不改，必须通过
    - 新增 `rerunning_the_same_search_does_not_reread_the_conversation`：同一个查询再跑一次（例如切换过滤条件），`conversation_seq` 不变；换了查询则变
  - 实现 `select_row`、`on_store_events`、`run_search` 的改动和 `detail:` 计时行。
- [x] **步骤 6：验收与收尾**
  - autoplay、`screenshot-detail.sh`、`make screenshot-detail`；修 `screenshot-clicks.sh`。
  - 跑 `make screenshot-detail`、`make screenshot-search`、`make screenshot-clicks`，截图归档到 `tmp/<date>-m4a-turn-cards/`，逐张用 `Read` 打开，和画稿 1–4 对照。
  - 按 D4 测打开速度：debug 和 release 各跑一次 `big-session`，再选一个 5MB 级的文件，取 `detail:` 日志行。
  - `cargo test --workspace`、`make parity`。
  - 文档：`kb/docs/app-architecture.md`（线程表加读取线程；事件表的 `Messages` 换成 `Conversation`；删掉"扫描期间请求要排队"里关于详情的部分），`kb/docs/gpui-kit-guide.md`（"滚动到某个子元素"一节换成 `list` / `ListState` 的做法：行表、`splice`、`scroll_to`、回调里怎么拿 `AppRoot`），以及总 plan 列出的其余文档。
  - known-issues：删掉"详情：索引扫描进行中选中一行，对话要等扫描结束才出现""详情：Conversation 只显示索引里的前 300 条……""搜索：工具调用与 thinking 里的命中在详情面板里看不到"三条；登记"表格 Prompts 列的计数和详情的回合编号口径不同"（D7：Prompts 列与 Python 脚本一致，把打断标记、部分非人工消息也算进去；写清楚哪些情况两边不一样）。
  - 勾选总 plan 的 M4a 一步，提交。

### 测试与验证

| 验收标准 | 怎么验 |
|---|---|
| 1 顶栏 | 步骤 3 `the_summary_counts…`；截图 `conversation` |
| 2 编号与 AUTO | 步骤 1 `turns_start_at…`、`notifications_form…`；截图 `special` |
| 3 折叠的卡片 | 步骤 3 的卡片、统计、折叠测试；截图 `conversation` |
| 4 刻度悬停与点击 | 鼠标悬停 autoplay 做不到：用 `screenshot-clicks.sh` 的 cliclick 点击一格刻度，再人工悬停看一次；点击后的状态由步骤 4 `stepping_to…` 同一套逻辑保证 |
| 5 步骤展开 | 截图 `steps-open`、`step-open`、`expand-all` |
| 6 特殊回合 | 步骤 1 的打断、API 错误、无回复、AUTO、compact 测试；截图 `special` 对照画稿 4 |
| 7 图片占位 | 步骤 1 `images_in_a_prompt…`；截图 `special` 或 `big-session`（70MB 文件里有 79 张图） |
| 8 命中跳转 | 步骤 4 的命中测试、步骤 5 的键盘测试；截图 `search-hit`、`next-hit` 对照画稿 3 |
| 9 无条数上限 | 步骤 1 读全文件；截图 `big-session` 滚到底（autoplay 里 `scroll_to` 最后一行） |
| 10 扫描期间 | 步骤 2 `a_conversation_is_served_before_the_first_scan_settles`；实机：`ASM_DB` 指向 `tmp/` 下的新路径冷启动，扫描中选中一行 |
| 11 打开速度 | `detail:` 日志行，release，`5f6eea99` < 200ms |
| 12 读取中 / 失败 | 步骤 2 `a_missing_transcript…`；骨架靠人工看一次（读取很快，截图未必抓得到） |

回归：`cargo test --workspace` 全过；`make parity` 21/21；`make screenshot-search` 的 7 张和 M3 时一致（详情区域除外）。

验收时的硬性规则（AGENTS.md）：不用 System Events / osascript / cliclick 发按键，不动剪贴板；截图前 `wake-screen`；截图必须用 `Read` 看过；手动起的 app 验证完要 kill。

### 风险与待定

| 风险 | 对策 |
|---|---|
| 70MB 文件读取 + 首帧超过 200ms（09-28 量的完整解码是 109–117ms，现在多了 tool_result 和 system 记录要解析） | 先量，看时间花在哪：解析就在 serde 之前用字节前缀跳过 `progress`、`file-history-snapshot` 这类不需要的记录（`parse::classify` 的做法，但要放行 `system`）；首帧就检查行表是不是一次建得太多 |
| 打断、API 错误、AUTO 的识别依据是 2026-10-26 的数据，Claude Code 升级可能改格式 | 步骤 1 用 `asm session --conversation` 在真实数据上核对；识别不到时退化为"no reply"，不会崩 |
| 旧版本记录没有 `stop_reason`，最终回复靠"最后一条纯文字消息"判断 | 与导出同一规则，导出的测试已覆盖 |
| `ListState::splice` 在视口上方插入行时，滚动位置是否保持 | 步骤 4 的测试先锁住；跳到命中时本来就会重新 `scroll_to` |
| 每次改搜索词都重读文件，70MB 约 100ms | 搜索本身有防抖；读取在后台线程，旧内容留在屏幕上；不够快再按总 plan D11 改成缓存 |
| 详情里的命中数和 Matches 列不同 | D5 已接受；不处理 |

## 实施结果与差异

2026-10-27 完成并验收：提交 `200a567`（core）、`2d8543b`（store 与转换）、`baaed71` + `be96d1a` + `f976862`（界面与返工 r1、r2），验收见 [验收总结](../sessions/2026-10-27-asm-m4a-accept.md)。截断常量（`ENTRY_CHARS` / `RESULT_CHARS` / `INPUT_FIELD_CHARS` / `HIT_LEAD_CHARS`）和折叠常量都没改。下面逐条列出实现与本 spec 不同的地方，理由在链接的出处。

core（[出处](../sessions/2026-10-26-asm-m4a-core.md#注意事项与自己的决定)）：

- `Matcher::pattern()` 返回带大小写标志的表达式（如 `(?i:needle)`），切换 `Aa` 也算换了搜索词
- 没有 `message.id` 的旧记录，同一条记录里的几段文字按记录序号合并
- `origin.kind` 不是 human 且有正文的消息，带 `isMeta` 也建 AUTO 段
- `turn_duration` 只累加到当前回合的用时，不结束回合
- 参数摘要另外支持 MultiEdit 的 `file_path`
- `ToolLine.text` 不带 `# ` / `$ ` 前缀，由使用方按 `ToolLineKind` 加（app 的 `InputLine` 已加好）
- `stringify` 实际在 `search/pyjson.rs`，`regex.rs` 改成 crate 内重导出

store 与转换（[出处](../sessions/2026-10-26-asm-m4a-store.md#和-spec-不同的地方)）：

- 读取线程读完立即 `conversation::build`，`StoreEvent::Conversation` 带 `Arc<ConversationView>` 而不是 `Arc<Conversation>`
- `Store::read_conversation` 多一个 `project_path` 参数；事件里用 `read`、`build` 两个耗时代替 `elapsed`
- `fold` 多一个 `hits` 参数
- 多了 `card_rows`（给 `splice` 用）和 `row_of`（命中位置 → 行号）
- compact 分隔线的时间和卡片同一规则：与上一张卡同一天只写 `HH:MM`
- 折叠步骤行的命中徽章是这一步里的命中次数；卡片标题和顶栏仍按命中位置数
- 输出为空（Read 图片、没有输出的命令）时结果摘要为空，不写"0 lines"
- 用时、大小、字数、路径缩写等 spec 没写死的格式见出处同节"另外几处"

界面与接线（[出处](../sessions/2026-10-27-asm-m4a-ui.md#自己的决定与-spec-的差异)、[返工 r1](../sessions/2026-10-27-asm-m4a-ui.md#返工-r1)）：

- Show steps 开关在卡头状态行右端（照画稿），不在卡底
- 命中在长文字深处时，把命中本身滚进视口（list autoscroll），不只停在所在行的顶端
- `Folded` 多一个 `short`，折叠处去掉结尾空行
- 当前命中位置里的高亮全部橙色；折叠步骤行的参数摘要不高亮，只有行尾徽章
- 刻度格 6 / 7px 宽、首尾相接，不是 8px 的透明区域
- `detail:` 行在列表后的 `canvas` paint 回调里打点，不用 `on_next_frame`；行里多一个 build 耗时
- 键盘测试 "parser" 的期望值仍是 `(1, 3)`：按命中位置数在 fixture 上也是 3
- `run_search` 比较最近一次请求的 pattern，不是屏幕上视图的 pattern
- 被截过的文本，折叠链接只数保留下来的部分（`Show 401 lines · cut, 20,000 of 24,999 chars`、`+ 47 more lines · cut, 3,000 of 16,319 chars`），整段显示时末尾加 `cut · showing X of Y chars`；为此 core 的 `Marked` 多一个 `kept`
- task-notification 的 AUTO 卡折叠时只显示 `<summary>`（摘要里没说到的 `<status>` 接在后面），Show all 打开整条原文；有命中在摘要外时直接整段显示（[返工 r2](../sessions/2026-10-27-asm-m4a-ui.md#返工-r2)）
- MCP 工具 `mcp__<server>__<tool>` 在步骤行、统计行和悬停说明里只写 `<tool>`，完整名写在展开后的 input 上方；长工具名把名称列撑宽（最多 168px），不截断（同上）
- 整段显示的文字同样去掉结尾空行，"Show all · N lines" 按去掉之后的行数算（同上）

验收（[出处](../sessions/2026-10-27-asm-m4a-accept.md#和-spec-不同的地方)）：

- `screenshot-clicks.sh` 的回合卡点击不在第 11 张，改为脚本末尾再启动一次、只预选 `5f6eea99`，截 16–20 共 5 张（Show steps、工具行、刻度悬停与点击）
- 验收标准 10 不用 detail autoplay（它等启动扫描结束才开始），改用一次 cliclick 在冷启动扫描中点一行
