---
created: 2026-10-27
tags:
  - m4b
  - detail-panel
  - code-block
  - history
  - agent-session-manager
---

# M4b 步骤 5 与其余界面：代码块、History 卡片、`[` / `]`、Metadata 的 Files、autoplay

## 协调者摘要（asm-m4b-extras）

- 结果：任务文件完成标准 1–5 全部满足。History 行改读 `history.jsonl` 显示 prompt 卡片；prompt / 过程文字 / 回复按段显示（代码块等宽框、行内代码）；`[` / `]` 按卡跳转并在顶栏提示；Metadata 加 Files；autoplay 加 `history-session`、`code-block`、`next-turn`，另加了一步 `metadata`
- 提交：代码、文档和本总结在同一个提交（`git log -1 -- kb/sessions/2026-10-27-asm-m4b-extras.md`）
- 测试：`make test` 全绿，core 166，app 127 通过、2 ignored。新增 app 测试 9 个：步骤 5 的 4 个、`[` `]` 的 2 个、`model.rs` 的 Files、History 卡片的构造和端到端各 1 个。红灯：`[` `]` 测试在实现前失败；去掉 Metadata tab 的判断后 `brackets_do_nothing_on_the_metadata_tab` 失败；Files 测试加入相对路径后先失败再修
- 截图：`tmp/2026-10-27-m4b-actions/extras/` 1–12（最终一轮），`extras/synthetic/`（合成数据：命中落在行内代码和代码块里）。用 Read 看过的见 §3。M4a 原有 8 步没有回退
- 对照画稿：画稿 5 的顶栏、虚线说明框、只有卡头的卡片一致；画稿 2 的代码块框一致，行内代码的字号与内边距做不到画稿的样子（§3）
- 和 spec 的差异：① `text_runs` 返回 `(区间, RunStyle)`，渲染用 `with_highlights` + `with_font_family_overrides`，没有自己拼 `TextRun`。② 折叠点落在代码块里时退到代码块之前；代码块就是开头时例外，折叠视图显示它的前几行。③ fence 行不显示，反引号保留但不画出来。④ `[` 先回到当前卡开头。⑤ History 卡片顶栏也显示 `[ ]` 提示。细节见 §2
- 未验证：History 卡片上的搜索命中和 `n` 跳转在界面上走不到（搜索模式不读 history，和 Python 脚本一致；见交给协调者第 3 条）
- 交给协调者：AGENTS.md 的 `screenshot-detail` 现在是 12 步；spec 同步 §2 的差异；PRD §5 和验收标准 8 的"能搜索"要定口径；一处疑似的悬停提示残留。见 §5

## 1. 做了什么

- **app `conversation.rs`**
  - `Segment::{Text { range, inline_code }, Code { range }}` 和 `segments(text)`：围栏代码块（3 个以上反引号开头，可以缩进；info string 里有反引号的不算；闭合要至少同样多的反引号）单独成段，没闭合的到文末。代码块两边的换行和空行算它的边，不算正文。行内代码只认单个反引号，不跨行，区间含两边的反引号
  - `fold_code`：给分段的文字用的 `fold`，fence 行照算行数，折叠点落在代码块里时退到代码块之前（例外见 §2 #2）。原来的 `fold` 不变，仍用于工具输出等
  - `RunStyle { code, backtick, hit }` 和 `text_runs(inline_code, hits)`：在行内代码、反引号、命中的每个边界切开，只返回要特殊画的段
  - `Folded` 加 `segments` / `short_segments`：只有 prompt、过程文字、回复（`folded()` 和任务通知的摘要）填；工具输出、thinking、API 错误、compact 摘要是空的，照旧整段画
  - History：`ConversationView.history`，`TurnCard.prompt_only`。`build` 遇到 `Conversation.history_only` 时每张卡由 `prompt_card` 生成：没有状态、刻度和卡尾，`end_note` 为空（不写 "no reply"）。顶栏摘要是 "N prompts from history.jsonl"。`push_rows` 对 prompt-only 卡只放 `Row::Head`
- **`turn_cards.rs`**
  - `marked` 按段画：正文段一个 `StyledText`（`run_highlight` 把 `RunStyle` 换成高亮，行内代码区间换等宽字体），代码段走 `code_block`（`#f7f7f9` 底、`hairline` 边框、圆角 4、内边距 6/8、等宽 11/15、不折行、裁掉超宽部分）。整段没有代码时照旧一个 `StyledText`
  - `reveal_marker` 多传一份第一段的 `TextLayout`，用来算文字的顶端
  - 顶栏：没有命中导航时，摘要后面显示 `[` `]` 键帽和 "jump between turns"（画稿 2 的 `.kbd` / `.hint`）
  - History：去掉 M4a 的说明句；顶栏照常显示；顶栏下面是画稿 5 的虚线说明框（`history_note`）；prompt-only 卡的卡头四角圆、没有下边线，也不画状态行
  - `AppRoot::step_turn`：只在详情面板显示、且停在 Conversation tab 时转给 `DetailState::step_turn`
- **`detail.rs`**
  - `DetailState::step_turn(forward)`：`]` 找视口顶行之后的第一个 `Row::Head`，`[` 找之前的最后一个（顶行是卡头但滚进去了，就算它），`scroll_to` 过去，不改展开状态
  - `scroll_to` 改成 `pub`（autoplay 用）
  - `files_open`（在 `forget()` 里复位）、`render_files`：Files 一行写 "12 files"，下面列前 10 个，多于 10 个时是 "Show all · N files" / "Show less"
  - `Load::Idle` 只表示没有选中
- **`main.rs`**
  - `NextTurn` / `PrevTurn` 绑在 `"DataTable"` 上的 `]` / `[`
  - `open_conversation` 不再特殊处理 History 行；`read_conversation` 按 `transcript_path` 选 `read_conversation` 或 `read_history`，搜索重跑时 History 行也重读
  - autoplay 在 `big-session` 后面加四步：
    - `history-session`：表里第一个 prompt 数不少于 3 的 History 行，没有就第一个 History 行
    - `code-block`：再选 `ASM_AUTOPLAY_SESSION` 的 session，稳定后挑第一个代码块离开头最近的回复，展开并把卡尾滚到顶端
    - `next-turn`：滚回顶部，`window.dispatch_action(NextTurn)` 两次
    - `metadata`：切到 Metadata tab
- **`model.rs`**：`SessionRow.files_touched`（项目里的文件写相对路径，排在前面，排好序、去重；其余的原样排在后面）和 `files_fmt`（"12 files" / "1 file" / "—"）
- **`store.rs`**：`read_history` 去掉 `#[allow(dead_code)]` 和 TODO
- **`style.rs`**：`code_block_bg`（#f7f7f9）、`inline_code_bg`（#ededf0）
- **脚本与文档**：`screenshot-detail.sh` 的 STEPS 和头部注释、Makefile、README 的说明改成 12 步。`app-architecture.md`：History 的读取流程、选中时的请求、`[` / `]` 的作用范围。`gpui-kit-guide.md`：新增"行内代码和代码块"一节；测试表加"测滚动位置"一行（要压矮窗口）；单键快捷键一句加上 `[` / `]`；截图一段改成 12 步

## 2. 自己的决定（含与 spec 的差异）

| # | 决定 | 原因 |
|---|---|---|
| 1 | spec 写的是 `text_runs(text, segment, hits, current, base_font, mono_font) -> Vec<TextRun>`，实际是 `text_runs(inline_code, hits) -> Vec<(Range, RunStyle)>`，渲染时转成 `with_highlights` 加 `with_font_family_overrides` | `TextRun` 要完整的字体和颜色，而它们是从父元素继承的（prompt 是 medium 字重，AUTO 卡是灰字，回复是正文色）。用高亮的话，GPUI 在布局时按继承来的样式算 run。纯函数仍然可测，测试含中文 |
| 2 | 折叠点落在代码块里时退到代码块之前（fence 行的行首）。例外：代码块前面没有文字时不退，折叠视图显示代码块的前几行，`short_segments` 里它没闭合，所以仍画成框。spec 原文是"折到代码块结束前的那一行" | 原文有歧义。照字面把整个代码块放进折叠视图，就起不到折叠的作用（贴进来的 200 行日志会整段显示）。往前退在代码块打头时会什么都不剩，而用户在 prompt 开头贴代码很常见。测试名照 spec，里面另有一条断言写明了这个例外 |
| 3 | fence 行（含 info string）不显示，代码块两边的换行和空行也不显示 | 和画稿 2 一致。代价是 fence 行上的命中（例如搜 `rust`）看不到，命中数照算 |
| 4 | 行内代码的反引号留在文字里，用 `fade_out: Some(1.)` 隐藏 | 区间保持是原文的区间。隐藏的反引号正好当灰底的左右留白（画稿是 3px，这里约 7px）。用透明色不行：GPUI 的高亮颜色是叠加的（`blend`），透明色叠上去等于没改，第一轮截图里反引号还看得见 |
| 5 | `[`：视口顶行在某张卡的中间时，先回到这张卡的开头，再按一次才到上一张 | 编辑器里"上一节"通常这样，不会跳过正在看的这张卡的开头 |
| 6 | History session 的顶栏也显示 `[ ]` 提示（画稿 5 没画） | `[` / `]` 在 prompt 卡之间同样能用，长的 History session 用得上；有这个提示也和普通 session 一致 |
| 7 | History 的说明是顶栏下面固定的一块虚线框，不随列表滚动；文字用 spec 的英文 | spec 写"一行灰字"，画稿 5 是 `.note` 虚线框，按画稿的样子画。不放进列表，是为了不给行表加一种行 |
| 8 | autoplay 多加一步 `metadata`，共 12 步 | Files 列表没有别的截图入口。spec 的测试表第 9 行要截 Metadata，accept 可以直接用 |
| 9 | `history-session` 选第一个 prompt 数不少于 3 的 History 行（spec 写的是第一个 History 行） | 只有一条 prompt 的卡片说明不了列表的样子；没有这样的行时退回第一个 |
| 10 | Files：没有斜杠开头的 `trackingPath` 算项目里的文件。同一个文件可能既记了相对路径又记了绝对路径，去重 | 实测 Claude Code 对工作目录下的文件记相对路径，对其余的记绝对路径（tmp 索引里 4702 条相对、545 条绝对）。fixture 里只有绝对路径，第一版只认绝对路径，截图里 `/Users/reorx/.ssh/config` 排到了最前面 |
| 11 | "Show all · N files" 点开后变成 "Show less" | 和回合卡的折叠链接一致 |
| 12 | 行内代码只认单个反引号、不跨行；两个以上的连续反引号既不开始也不结束 | spec 只要求单个反引号；双反引号的写法很少见，按普通文字显示也不会出错 |

## 3. 截图与画稿对照

目录 `tmp/2026-10-27-m4b-actions/extras/`。`ASM_DB=tmp/2026-10-27-m4b-extras/index/run1.sqlite`，第一次运行是冷启动。`ASM_AUTOPLAY_SPECIAL=0f2d9ee5`，其余变量用默认值。

| 截图 | 内容 | 和画稿的差异 |
|---|---|---|
| `9-history-session.png` | `4ea4b5d9`（opus55，5 条 prompt）。顶栏是 "5 prompts from history.jsonl" 加 `[ ]` 提示，下面是虚线说明框，5 张只有卡头的卡片。★、Copy resume、Finder、导出置灰 | 画稿 5：说明是英文（spec 的措辞）；顶栏多了提示（§2 #6）；多了 ★。第一张卡的时间带日期，在 58px 的标签列里折成两行，这是 M4a 的行为 |
| `10-code-block.png` | `35fec037` 第 8 张卡的回复：两个代码块是浅灰底细边框的等宽框，中文在框里回落到系统字体 | 画稿 2 的 `.block` 一致 |
| `11-next-turn.png` | 滚回顶部后按两次 `]`，第三张卡（TURN 3）在顶端，顶栏有 `[ ]` 提示。回复里的 `ListenPort` 是等宽字浅灰底，反引号不显示（放大检查过） | 画稿 2 的 `.ic`：字号和正文相同（画稿是 11px），留白约 7px（画稿是 3px）。`TextRun` 不能单独设字号 |
| `12-metadata.png` | 同一个 session 的 Metadata：Files 写 "47 files"，先列项目里的文件（`AGENTS.md`、`deploy/…`），共 10 行，然后是 "Show all · 47 files" | 没有画稿 |
| `synthetic/5-search-hit.png` | 合成数据（`make_code_fixture.py` 生成的 `66666666`），正则 `flaky\|parse\.rs`：prompt 里行内代码 `parse.rs` 上的当前命中是橙底，字体仍是等宽；prompt 和回复的代码块里，命中照常高亮；超宽的一行在框的右边被裁掉，没有折行 | 验收标准 11 的证据 |
| 1–8 | M4a 原有 8 步：选中、展开步骤、展开一步、全部展开、搜索命中、下一个命中、特殊回合、70MB | Read 看过 1、3、5、6、7、8：没有回退。没有搜索时顶栏多了 `[ ]` 提示，搜索时位置让给命中导航；过程文字里的行内代码（`~/.ssh/config`）也按段显示 |

截图共跑了三轮。第一轮里反引号还看得见，用 `fade_out` 修了；第二轮放大检查了行内代码；第三轮（最终）在 Files 的修正之后。1–12 都出自最终一轮。9、10 在第一轮看过，两轮之间这两张用到的代码没有变化，最终一轮里 Read 的是 12。

`extras/hit-in-code/` 是一次没用上的尝试：搜索会选中结果里最新的 session，也就是正在写入的这个会话，选不到想要的那个。`extras/metadata/` 是 `make screenshot` 那一轮，选中的 session 没有改过文件，显示 "Files —"。

日志里的打开速度（debug）：History session `4ea4b5d9` 首帧 27.9ms；`35fec037`（15MB）46.9ms；`5f6eea99`（70MB）42.9ms。

## 4. 给后来的执行者

- History 卡片就是普通的回合卡，只是带 `prompt_only`。行表里一张卡只有一行，`row_of(Part::Reply)` 永远找不到，也不需要找
- 分段只在构造卡片时算一次（`Folded::segmented`）。渲染时只有含代码的文字才按段切字符串；整段没有代码时走原来的单个 `StyledText`
- 键盘测试里测滚动位置，要先 `cx.simulate_window_resize` 把窗口压矮。fixture 的对话很短，整个放得下，`list` 在下一帧会把滚过头的位置拉回顶端
- 合成数据的脚本 `tmp/2026-10-27-m4b-extras/make_code_fixture.py <新目录>`：复制 core fixture，再加一个最新的 session，带行内代码、代码块和超宽行。配合 `ASM_CLAUDE_DIR` / `ASM_DB` / `ASM_FAVORITES_DIR` / `ASM_TRASH_DIR` 指到 `tmp/` 下，可以截出确定的画面

## 5. 交给协调者

1. **AGENTS.md** 开发命令一节：`make screenshot-detail` 现在是 12 步，在 8 步后面加 `history-session`（第一个 prompt 数不少于 3 的 History 行）、`code-block`（`ASM_AUTOPLAY_SESSION` 的 session，展开代码块离开头最近的回复）、`next-turn`（从顶部按两次 `]`）、`metadata`
2. **spec 同步**：§2 的 #1（`text_runs` 的签名与渲染方式）、#2（折叠与代码块的规则）、#3、#4、#5、#6、#7、#8、#10
3. **History 的搜索走不到**：PRD §5 写"搜索命中照常高亮，可以用 n / ⌘G 在 prompt 之间跳"，验收标准 8 写"能搜索和跳转"。但搜索模式只读记录文件，不读 `history.jsonl`（Python 脚本也是这样，`claude_session_search.py` 的帮助文本写着 "Search mode reads neither the history nor subagent transcripts"）。所以搜索时表格不会列出 History session，界面上看不到它的卡片被标命中。读取线程会标（`store.rs` 的 `history_prompts_are_served_for_a_history_session`），`[` / `]` 能用。两种处理：把 PRD 和验收标准改成"不参与搜索"；或者另开任务让搜索覆盖 history prompt，但这要两边一起改才能保持 parity。可以登记到 known-issues
4. **疑似的悬停提示残留**（没有深究，属于 actions 的范围）：autoplay 期间鼠标一直停在 Copy resume 的位置，换到 History session 以后，按钮已经置灰，提示仍是上一个 session 的 "Copy resume command ⇧⌘C"，不是 "The transcript is gone"。可能要等鼠标移动才会刷新。见 `9-history-session.png` 右上角
5. `kb/docs/product-prd.md`（归 accept）：第 46 行"History 的 prompt 列表、代码块框"，第 147 行"记录文件已被清理的 History session 显示一句说明"，第 156 行，这些都已实现
