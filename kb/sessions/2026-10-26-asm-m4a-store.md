---
created: 2026-10-26
tags:
  - m4a
  - store
  - conversation
  - turn-cards
  - agent-session-manager
---

# M4a 步骤 2、3：读取线程与回合卡转换

## 协调者摘要（asm-m4a-store）

- 结果：步骤 2、3 完成。`store.rs` 新增 `asm-reader` 线程、`Store::read_conversation`、`StoreEvent::Conversation`；app 新文件 `conversation.rs`（卡片类型、`build`、`rows`、`card_rows`、`row_of`、`fold`）。界面没有接入：`main.rs` 只加了 `mod conversation;` 和一个忽略 `Conversation` 事件的分支（不加编译不过）
- 提交：代码、`kb/docs/app-architecture.md` 和本总结同一个提交
- 测试：`make test` core 160、app 95 全过（app 原 80 个 + 新 15 个：store 4 个、conversation 9 个 spec 要求的加 2 个补充），2 个 ignored（`real_index_timings` 和新的 `real_transcript_timings`）；旧测试一个没删没改名。红灯日志与最终日志在 `tmp/2026-10-26-asm-m4a-store/`（`store-red.log`、`conversation-red.log`、`make-test.log`）
- 测量（release，`build` + `rows` 全部折叠，5 次中位数）：`5f6eea99`（70MB）0.17ms；回合数最多的 `d8ab2969`（30 个回合）0.31ms；卡片最多的 `4e5802b6`（14 回合 + 45 AUTO）1.11ms；工具调用最多的 `35fec037`（481 次）1.02ms。读取 4–21ms。日志 `tmp/2026-10-26-asm-m4a-store/release-timings.log`
- 自己的决定（与 spec 的差异，理由见下文"和 spec 不同的地方"）：① `build` 在读取线程上做，事件带 `Arc<ConversationView>` 而不是 `Arc<Conversation>`；② `read_conversation` 多一个 `project_path` 参数，事件用 `read` / `build` 两个耗时代替 `elapsed`；③ `fold` 多一个 `hits` 参数；④ 多给了 `card_rows`、`row_of`；⑤ compact 分隔线的时间和卡片同一规则（画稿里总带日期）；⑥ 步骤行的命中徽章是命中次数，卡片标题的是命中位置数；⑦ 只有图片的工具输出，结果摘要为空
- 未验证：没有起 app、没截图（本任务不涉及界面）；打开速度的首帧时间要等 asm-m4a-ui 接线后由验收量
- 交给协调者：① asm-m4a-ui 接线后删掉 `conversation.rs` 顶部的 `#![allow(dead_code)]` 和 `store.rs` 里三处 `#[allow(dead_code)]`；② spec 要同步的是上面 ①–⑦ 和下文常量表；③ 可登记的小问题：core 的 `ToolResult` 不记录输出里的图片，Read 图片的步骤行看不出读到了什么（下文"遗留"）

## 接口清单（给 asm-m4a-ui）

- `Store::read_conversation(&self, session_id: String, path: PathBuf, project_path: String, matcher: Option<Matcher>) -> u64` — 返回本次 `seq`；之前没开始读的请求作废
- `StoreEvent::Conversation { seq: u64, session_id: String, conversation: Result<Arc<ConversationView>, String>, read: Duration, build: Duration }` — 已经转换好的卡片；`Err` 是给人看的一行说明（含路径）
- `conversation::build(conv: &Conversation, project_path: &str, today: NaiveDate) -> ConversationView` — 读取线程已经调过，界面一般不用再调；测试里可以用
- `conversation::rows(view: &ConversationView, open: &HashSet<usize>) -> Vec<Row>` — 整张行表，`open` 是展开的卡片序号
- `conversation::card_rows(ix: usize, card: &Card, open: bool) -> Vec<Row>` — 一张卡的行，给 `ListState::splice` 用
- `conversation::row_of(rows: &[Row], place: &HitPlace) -> Option<usize>` — 命中位置所在的行（收起的回合里的步骤落在 Head 上）
- `conversation::fold(text: &str, hits: &[Range<usize>], max_lines: usize, max_chars: usize) -> Option<usize>` — 折叠处的字节位置；`build` 已经算进每个 `Folded.fold`
- `enum Row { Head(usize), Step(usize, usize), Tail(usize), Compact(usize) }`
- `enum Card { Turn(TurnCard), Compact(CompactCard) }` — 与 core 的 `ConvItem` 一一对应、同序号，所以 `HitPlace.item` 就是卡片序号
- `struct ConversationView { summary, cards: Vec<Card>, hits: Vec<HitPlace>, pattern: Option<String>, bytes: u64 }`
- `struct TurnCard { label, auto, origin, time, status: Status, meta, errors, end_note, prompt: Folded, attachments, ticks: Vec<Tick>, stats, steps: Vec<StepView>, tail: Tail, hits: usize, hits_fmt }`
- `struct CompactCard { label, summary_label, summary: Option<Folded>, hits }`
- `struct Folded { text, hits: Vec<Range<usize>>, fold: Option<usize>, more }`
- `struct StepView { kind: TickKind, time, label, summary, meta, error, hits: usize, hits_fmt, body: StepBody }`
- `enum StepBody { Tool { input: Vec<InputLine>, output: Option<Folded> }, Thinking(Folded), Text(Folded) }`；`struct InputLine { kind: InputKind, text, hits }`；`enum InputKind { Description, Command, Field }`
- `enum Tail { Reply { time, text: Folded }, Interrupted { time }, ApiError { time, text: Folded }, Empty }`
- `struct Tick { kind: TickKind, error: bool, hit: bool }`；`enum TickKind { Tool, Text, Thinking }`；`enum Status { Ok, Failed, Off }`

字符串字段都是 `SharedString`，渲染直接用。

## 和 spec 不同的地方

| # | spec | 实际 | 原因 |
|---|---|---|---|
| 1 | 事件带 `Arc<Conversation>`，`main.rs` 在 `on_store_events` 里调 `build` | 读取线程读完立即 `build`，事件带 `Arc<ConversationView>` | 转换不占主线程（项目的核心诉求是界面响应）；原始 `Conversation` 在读取线程上释放，内存里不同时留两份文本。代价：`today` 取读取那一刻的日期，跨午夜不重算（无关紧要） |
| 2 | `read_conversation(session_id, path, matcher)`；事件里一个 `elapsed` | 多一个 `project_path`；事件里 `read` 和 `build` 两个耗时 | `build` 要项目目录把路径改成相对路径；分开两个耗时，`detail:` 日志能看出时间花在哪 |
| 3 | `fold(text, max_lines, max_chars)`，"折叠处之后有命中时返回 None" | `fold(text, hits, max_lines, max_chars)` | 要判断命中位置就得把命中传进来 |
| 4 | — | 多了 `card_rows`、`row_of` | `splice` 要一张卡的行，跳到命中要找行号；放在这里和 `rows` 共用同一套规则，界面不用再写一遍 |
| 5 | compact 分隔线写"context compacted · 10-17 22:05"（画稿 4 里总带日期） | 和卡片同一规则：与上一张卡同一天只写 `HH:MM` | spec 第 2 节的日期规则写的是"卡"，分隔线跟着它更一致；画稿里那张卡恰好不是今天的。想要分隔线总带日期，改 `build` 里的一行即可 |
| 6 | "折叠着的步骤行如果含命中，行尾显示命中数" | `StepView.hits` 是这一步里的命中次数（输入和输出加起来）；`TurnCard.hits` 和顶栏仍按命中位置数 | 一步就是一个命中位置，按位置数徽章永远是 1，没有信息量；画稿 3 里 Grep 那一行显示 1，但它展开后有 4 处高亮，画稿本身不一致 |
| 7 | 结果摘要"其余 `N lines`" | 输出为空（Read 图片、没有输出的命令）时结果摘要为空 | Read 图片的结果在 core 里没有文字，写"0 lines"会误导；70MB 文件里 79 次 Read 都是这样 |

另外几处 spec 没写死、我定的格式：

- 用时：`<1s`、`45s`、`1m 02s`、`1h 02m`（小时下的分钟也补零）
- 大小：`640 B`、`4.1 KB`（10 KB 以下一位小数）、`860 KB`、`1.2 MB`；字数：`880 chars`、`1.2k chars`、`21k chars`
- 一整行太长被字数折叠时，链接写 `Show all · 2.0k chars`（按行数写会是"1 lines"）；工具输出写 `+ 2.1k more · 4.1 KB`
- 工具输出的 `+ 40 more lines · 4.1 KB`：行数是原文行数减去已显示的行数，大小是整份输出的大小
- 步骤、回复、打断、API 错误的时间：与所在卡片同一天只写 `HH:MM`，否则带日期
- thinking 和过程文字的一行摘要：第一行非空文字，最多 120 字，后面还有内容就加 `…`
- 路径缩写：在项目目录下写相对路径，项目目录本身写 `.`，家目录下写 `~/…`；超过 60 字时保留文件名和能放进 2/3 宽度的上级目录，再尽量保留开头几级，中间写 `…`（`~/.cargo/registry/…/gpui-component-0.6.1/src/date_picker.rs`，和画稿一致）。只缩写 Read / Edit / Write / MultiEdit / NotebookEdit 的参数和 Grep 参数里的 path 部分，Bash 命令里的路径原样
- 没有步骤、也没有用时的卡片，`meta` 为空字符串

## 常量

| 常量 | 值 | spec | 说明 |
|---|---|---|---|
| `PROMPT_LINES` | 4 | 同 | |
| `REPLY_LINES` | 6 | 同 | 回复和过程文字 |
| `OUTPUT_LINES` | 12 | 同 | |
| `FOLD_CHARS` | 900 | 同 | 对工具输出也生效：每行很长的输出不到 12 行就会折叠 |
| `ERROR_LINES` | 2 | "最多 2 行" | API 错误 |
| `PATH_CHARS` | 60 | 未写 | 路径中间省略的阈值，按 580px 详情面板里参数列的宽度估的 |
| `OPENING_CHARS` | 120 | 未写 | thinking / 过程文字的一行摘要 |
| `SIZE_SHOWN_BYTES` | 512 | "不小于 512 B" | |

core 的截断常量没动。

## 给 asm-m4a-ui 的注意事项

- 读取线程已经按 `seq` 取代，界面仍要核对：`seq == self.conversation_seq` 且 `session_id` 是当前选中的才用
- "搜索词是否变了"比的是 `view.pattern` 和新 `Matcher::pattern()`，不比原始查询（大小写切换会改变 pattern）
- 命中自动展开的规则一半在这里：`Folded.fold` 在折叠处之后有命中时已经是 `None`，所以 `unfolded` 集合只需要记用户点开的那些。回合和步骤的展开要界面按 `HitPlace` 做
- 刻度当前命中（橙圈）是界面状态；`Tick.hit` 只表示含命中（黄圈）
- 刻度悬停的说明行由 `StepView` 的字段拼：`{第几格} / {总格数}  {label}  {summary}  {meta}`，`error` 为真时 `meta` 用红色
- `Tail::Empty`：没有回复也没有异常（灰灯、"no reply"）。Tail 行还要放 Show steps 开关；`steps` 为空时不放
- AUTO 卡的 `origin` 显示在 prompt 上方；`session start` 的 AUTO 卡 prompt 是空的
- `InputLine.text` 已带 `# ` / `$ ` / `key: ` 前缀，命中区间已跟着前缀平移；`InputKind::Description` 用灰色
- 读取失败的 `Err` 文字形如 `open /path/x.jsonl: No such file or directory (os error 2)`

## 测试

store（`crates/app/src/store.rs`）：

- `a_conversation_is_served_on_request`：fixture S1 读成 4 张卡、"4 turns · 3 tool calls"；带 `Matcher` 再读，`view.pattern` 等于 `matcher.pattern()`、有命中
- `a_conversation_is_served_before_the_first_scan_settles`：测试先用另一个连接 `BEGIN IMMEDIATE` 占住索引，store 线程的扫描只能等；这时请求的对话 3 秒内到达（rusqlite 的忙等是 5 秒，排在扫描后面的实现过不了），之前没有 `ScanDone` / `Failed`；放开锁后扫描照常完成、流出 8 行。连跑 15 次稳定
- `a_newer_request_supersedes_the_one_before`：直接在测试线程上跑 `serve_reads`，队列里 3 个请求只读最后一个；再经 `Store` 连发两个，最后一个事件是新 `seq`，之前的只可能是旧 `seq`
- `a_missing_transcript_is_reported_instead_of_crashing`：不存在的文件得到带文件名的 `Err`，之后的请求照常

conversation（`crates/app/src/conversation.rs`），spec 的 9 个：`the_summary_counts_turns_auto_segments_and_tool_calls`、`a_card_shows_steps_duration_errors_and_how_the_turn_ended`、`stats_list_tools_by_count_then_text_and_thinking`、`ticks_follow_the_steps_and_mark_errors_and_hits`、`a_tool_step_summarizes_its_argument_and_result`、`folding_counts_source_lines_and_caps_one_long_line`、`a_hit_inside_a_fold_opens_it`、`times_carry_the_date_when_the_day_changes`、`a_closed_turn_is_a_head_and_a_tail_and_an_open_turn_lists_its_steps_between`；补充的 2 个：`thinking_and_process_text_show_their_opening_and_thinking_its_length`、`attachments_are_labeled_with_their_kind_and_size`。

先写测试和 `todo!()` 桩，确认 4 + 11 个失败（日志 `store-red.log`、`conversation-red.log`），再实现。实现后又发现 `elide_middle` 会去掉绝对路径开头的 `/`，补了断言再修。

## 测量

`real_transcript_timings`（ignored）：

```sh
ASM_BENCH_TRANSCRIPTS=<路径>,<路径> cargo test --release -p agent-session-manager real_transcript_timings -- --ignored --nocapture
# ASM_BENCH_REGEX=<正则> 带搜索读；ASM_BENCH_PROJECT=<项目目录>；ASM_BENCH_DUMP=1 打印每张卡的字符串
```

本机 release，页缓存热，每个文件 5 次（毫秒）：

| 文件 | 大小 | 卡片 | 步骤 | 读取 | build | rows（折叠） | build + rows 中位数 | 全部展开的 rows |
|---|---:|---|---:|---:|---:|---:|---:|---:|
| `5f6eea99` | 70.0 MB | 2 turns · 112 tool calls | 121 | 18.1–18.8 | 0.16–0.33 | <0.001 | 0.17 | 125 行 0.001 |
| `d8ab2969` | 4.4 MB | 30 turns · 143 tool calls | 173 | 3.8–3.9 | 0.29–0.33 | <0.001 | 0.31 | 233 行 0.001 |
| `4e5802b6` | 35.2 MB | 14 turns · 45 auto · 465 tool calls | 521 | 20.7–21.0 | 1.11–1.22 | <0.001 | 1.11 | 642 行 0.002 |
| `35fec037` | 15.7 MB | 13 turns · 4 auto · 481 tool calls | 681 | 13.9–14.6 | 0.95–1.06 | <0.001 | 1.02 | 719 行 0.002 |

带正则 `cargo (test|build)|pnpm|image` 读（`4e5802b6` 有 101 个命中位置）数字基本不变。debug 下 `build` 约为 release 的 3–5 倍（`4e5802b6` 5.5ms）。200ms 的预算里，读取加转换最多用了约 22ms，剩下的是首帧布局。

"回合数最多"的选法：用 `asm index` 在 `tmp/2026-10-26-asm-m4a-store/index/fresh.sqlite` 建一份一次性索引（真实索引没动；`~/Library/Application Support/dev.reorx.AgentSessionManager/index.sqlite` 还是 09-30 的旧 schema），取 Prompts 最多的 6 个 session 用 `asm session <路径> --conversation` 数回合。Prompts 列比回合数宽（含通知等），回合数超过 30 的 session 的 Prompts 必然不小于 30，这 6 个已经覆盖。`4e5802b6` 的 Prompts 是 61，回合只有 14，其余 45 是 AUTO。

在真实数据上用 `ASM_BENCH_DUMP=1` 看过 `d8ab2969`、`5f6eea99`、`4e5802b6` 的全部卡片字符串：日期、用时、统计行、结果摘要、相对路径、附件标签、"Show all"、AUTO 来源都符合预期。

## 遗留

- core 的 `ToolResult` 只有文字输出；Read 图片、截图这类结果的输出为空，步骤行的结果摘要也就为空。要显示成"image · 230 KB"，得在 core 的 `result()` 里像 prompt 的附件一样记下图片（`ToolResult` 加一个 `attachments`）。不影响 M4a 的验收标准，是否做由协调者定
- `today` 在读取线程上按读取时刻取，app 跨午夜开着时，已经显示的卡片日期不会重算；重新选中就对了

## 相关文档

- [M4 总 plan](../plans/2026-10-26-m4-detail-panel.md) — 回合、命中位置、D10–D12
- [M4a spec](../plans/2026-10-26-m4a-turn-cards-spec.md) — 步骤 2、3 的测试与 app 逻辑变动
- [core 步骤总结](2026-10-26-asm-m4a-core.md) — `read_conversation`、`Matcher::pattern()`、截断常量
- [运行时架构](../docs/app-architecture.md) — 线程表、事件表、"详情的读取"
