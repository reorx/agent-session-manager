---
created: 2026-10-26
tags:
  - m4
  - detail-panel
  - conversation
  - turn-cards
  - plan
  - agent-session-manager
---

# M4：详情面板按回合显示完整对话，并提供交接操作

取代 `2026-09-28-m4-detail-panel.md`（那份写于 m3 时期，其第 0 节记录了 2026-10-24 复核时的决定，已全部并入本 plan）。本 plan 是总 plan，两期的细节各在一份 spec 里。

## PRD

### 背景

用户用这个应用回看本机的 Claude Code session：这个 session 做了什么，哪一轮出了问题，最后交代了什么；看完以后接着这个 session 继续干活（resume），或者把它交给别人。

现在详情面板的 Conversation 只显示索引里前 300 条 user / assistant 文本，每条截到 4000 字：

- 看不到工具调用和 thinking。想知道 Agent 在哪一轮跑了什么命令、哪一步报了错，只能去翻原始记录文件。
- 搜索命中在工具输出或 thinking 里时，表格说这个 session 有命中，详情里却找不到。
- 长 session 被截断，后半段看不到。
- 冷启动或重建索引的 3–4 秒里选中一行，要等扫描结束对话才出现。
- 复制 resume 命令、复制 session ID、找到记录文件、导出 TXT，都没有入口，只能在 Metadata 里手动选文字。

### 目标

选中任何一个 session，200ms 内看到按回合组织的完整对话。每个回合一眼看得出：用户问了什么，Agent 做了几步，有没有出错，最后答了什么。搜索命中不管在 prompt、工具的输入输出、thinking 还是回复里，都能跳过去。看完以后，点一下就能拿到恢复对话或交接所需的东西。

### 画稿

https://claude.ai/artifact/M31PYmLyK2m5n8Bizyn9fz （私有，2026-10-24 用户确认）。共 6 张，都以当前界面为基准：

| # | 画面 | 内容 | 属于 |
|---|---|---|---|
| 1 | Main | 完整窗口，回合卡全部折叠 | M4a（标题行的操作按钮属于 M4b） |
| 2 | Expanded | 展开 TURN 3 的步骤，出错的 Bash 那一步展开；回复里有代码块 | M4a（代码块的框属于 M4b） |
| 3 | Search | 搜索 `api_usage`，命中在 Grep 的输出里 | M4a |
| 4 | Special | 长 prompt 和图片、compact、AUTO、被打断、API 出错、没有回复 | M4a |
| 5 | History | 记录文件已被清理的 session：只有 prompt 的卡片，部分操作按钮置灰 | M4b |
| 6 | Actions | 表格行的右键菜单 | M4b |

画稿之后又定了三件事，画稿与本 plan 不一致时以本 plan 为准：

- ★ 收藏按钮并入标题行的操作按钮组，排第一个；
- 右键菜单在现有的"收藏"、"在 Finder 中显示"两项上扩展，全部改用英文；
- 收藏过、但原始记录文件已被 Claude Code 清理的 session 不能 resume，复制 resume 命令的入口置灰。

### 共用概念

两期都用到这些概念，spec 里不再重复。

- **回合（turn）**：从用户的一条真实输入开始，到下一条真实输入之前为止。"真实输入"的判断和 TXT 导出一致：harness 注入的内容、compact 摘要、后台任务通知这类不是人发起的消息都不算；同一条记录在文件里重复出现只算一次。只有从真实输入开始的回合才编号，标题写 `TURN n`，n 从 session 开头数起。
- **AUTO 卡**：不是人发起的段落（后台任务完成的通知、自动续跑、其他 Agent 发来的消息等）也显示成卡片，样式和回合卡一样，但标题写 `AUTO` 加来源标签，不占编号。
- **compact 分隔线**：上下文被压缩的位置显示一条横线，写"context compacted"和时间。压缩摘要折叠在分隔线上，可以展开。
- **步骤（step）**：回合里 Agent 做的每一件事，按发生顺序排：一次工具调用、一段过程文字（Agent 在工具调用之间说的话）、一段 thinking。最终回复不算步骤。
- **最终回复**：回合结束前 Agent 的最后一段纯文字，单独显示在卡片底部。
- **回合状态**：绿灯表示正常结束；红灯表示被用户打断或 API 出错；灰灯表示没有最终回复（例如 session 还在进行，或者进程中途退出）。
- **刻度条**：卡片里的一行刻度，一格代表一步。工具调用是一条竖杠，出错的是红色；过程文字是实心点；thinking 是空心点。
- **命中位置**：一个 prompt、一个步骤、一段最终回复、一段压缩摘要，只要含有搜索词，就算一个命中位置。"▲ 3 / 9 hits ▼"按命中位置计数和跳转。
- **操作**：复制 resume 命令、复制 session ID、在 Finder 中显示记录文件、打开项目目录、导出 TXT、收藏 / 取消收藏。

### 分期

| 期 | spec | 内容 |
|---|---|---|
| M4a | [回合卡与完整对话](2026-10-26-m4a-turn-cards-spec.md) | 直接读记录文件；回合卡的折叠与展开、刻度条、prompt 和回复按行折叠、图片占位；特殊回合；命中高亮与跳转；扫描期间照常显示；打开速度。对应画稿 1–4 |
| M4b | [操作与其余条目](2026-10-26-m4b-actions-and-extras-spec.md) | 标题行操作按钮、右键菜单、`⌘C` / `⇧⌘C`、导出 TXT、History session 的 prompt 列表、Metadata 加修改过的文件、`[` / `]` 按回合跳转、代码块显示。对应画稿 2、5、6 |

M4a 先做，合并后再开始 M4b。两期各自验收、各自提交。

### 不做的事

- 完整的 Markdown 渲染（标题、列表、表格、加粗）：回复按纯文本显示，M4b 只把代码块单独框出来
- 代码语法高亮、shell 命令的自动排版
- 查看 subagent 的对话：和"subagent 显示为子行"一起放到 M5
- 在应用里直接拉起终端执行 resume：放到 M5，依赖终端应用的设置
- 对话随记录文件的追加实时刷新：M5
- 每个回合的费用
- 按消息类型过滤（原来的 User / Assistant / Tools / Thinking 开关）：由回合卡的折叠取代，去掉
- 表格 Prompts 列的计数口径不变（仍与 Python 脚本一致），它和回合编号的差异在 M4a 登记到 known-issues

### 整体验收标准

- 选中本机最大的记录文件（70MB）对应的 session，从选中到对话出现不超过 200ms（release 构建）
- 任意 session 的全部回合都能看到，没有条数上限
- 正则搜索命中工具输出时，详情里能看到这个命中并跳过去
- 索引扫描进行中选中一行，对话照常出现
- 复制的 resume 命令粘贴到终端能直接恢复这个 session
- 已有功能不回退：表格、搜索、收藏，以及 `n` / `⇧N` / `⌘G` / `⇧⌘G` 的命中跳转

## Tech Design

### 现状

- 详情面板在 `crates/app/src/detail.rs`：`DetailState` 持有索引行 `raw: Vec<MessageRow>` 和转换后的 `ConversationView`，用 `ScrollHandle::scroll_to_top_of_item` 按子元素序号定位命中。数据来自 `StoreRequest::Messages` → `Worker::messages`（`store.rs`）→ `Db::messages`，经 `model::conversation_from` 截取（`CONVERSATION_CAP = 300`、`MESSAGE_CHARS = 4000`、`shown_text` 移动截断窗口）。
- core 里可复用的：`transcript.rs` 的导出规则（`human_text` 判断真实输入、`Turn::assistant` 判断最终回复、按 uuid 去重），`search::Matcher`（文本模式与正则模式共用的高亮器），`search::regex` 的 `ToolLine` / `ToolLineKind`。
- 收藏（`2026-10-26-favorites.md`）：`discover_all` 让收藏副本优先，`SessionSummary.file_path` 指向胜出的那一份，所以详情直接读 `SessionRow.transcript_path` 就能读到收藏副本；`favorites::original_exists` 判断原始记录文件是否还在。

### 跨期的关键决策

D1–D8 是 2026-10-24 复核时与用户定下的，D9 起是写本 plan 时的技术选择。

| # | 决定 | 理由；被否决的备选 |
|---|---|---|
| D1 | 列表用 GPUI 核心的 `list` + `ListState` | `v_virtual_list` 要预先给出每项高度（`gpui-base-0.6.1/src/virtual_list.rs`），自动换行的文本算不出来 |
| D2 | 拆成 M4a / M4b | R6 条目多，验收指标只取决于数据通路和列表 |
| D3 | 不用 `TextView` 和 Markdown；M4b 做轻量分段（代码块用等宽框，其余纯文本）；折叠按源文本行数 | `TextView::markdown` 没有传入高亮区间的接口，`max_lines` 也要先布局全文，折叠省不了开销 |
| D4 | 打开速度的起点是 `select_row`，终点是带对话内容的首帧绘制完成；基准文件 `5f6eea99`（70MB，09-14 后不再变化），另报一个 5MB 级文件；debug / release 各测一次，以 release 为准 | 立项时的"30MB"已不是本机最大 |
| D5 | 详情里的命中是页内查找语义：所有显示的内容都标，不受 Scope / Labels 限制 | 实现简单；代价是计数可能和 Matches 列不同 |
| D6 | 拉起终端放到 M5 | 依赖 R11 终端设置与 AppleScript 权限 |
| D7 | 回合切分沿用 TXT 导出的规则；Prompts 列不动 | Prompts 列要和 Python 脚本逐字比对（`make parity`） |
| D8 | resume 命令是 `cd <项目目录> && claude --resume <id>`，目录不存在时退回不带 `cd` | 实现前要验证（见 M4b spec 的风险） |
| D9 | core 新模块 `claude/conversation.rs`，类型叫 `Conversation`，不用 transcript 这个名字 | `transcript.rs` 已经是 TXT 导出 |
| D10 | 读记录文件放在 `store.rs` 内的第二条线程，按 `seq` 取代旧请求 | 扫描占着 store 线程时请求不用排队；取代机制照搬 `search.rs` |
| D11 | 搜索词变化时重读文件重新标记，不在内存里缓存全文 | 页缓存热时 70MB 约 100ms，且在后台线程；内存里只留截断后的文本。实测不够快再换成缓存 |
| D12 | 列表的一项是卡片的一段（卡头、一步、卡尾、compact 分隔线），不是整张卡 | 展开 200 步的回合不会变成一个巨大的项，每帧只布局看得见的部分；命中能定位到具体某一步 |

### 各期之间的依赖

M4b 建在 M4a 之上：读取线程和 `StoreRequest` / `StoreEvent` 的通路、列表的行模型（`[` / `]` 按卡头跳转，History 卡片复用卡头）、按行折叠的纯文本（M4b 在它上面加代码块分段）。M4a 的列表行模型要为这几处留好位置，但不实现它们。

### 实施步骤

- [x] M4a：按 [spec](2026-10-26-m4a-turn-cards-spec.md) 实施、验收、提交（2026-10-27，验收见 `kb/sessions/2026-10-27-asm-m4a-accept.md`）
- [ ] M4b：按 [spec](2026-10-26-m4b-actions-and-extras-spec.md) 实施、验收、提交

### 每期完成后都要更新的文档

- `AGENTS.md`"当前状态"
- `kb/docs/product-prd.md`：状态表、§4.5 详情面板、§6 性能目标
- `kb/plans/2026-09-08-agent-session-manager-prd.md` §3 里程碑表（M4 一行）
- `kb/known-issues.md`：删掉这一期解决的条目，登记新发现的
- `kb/sessions/` 写一份 session 总结（实测数字、和 spec 的差异都记在那里）
- M4a 另外要更新 `kb/docs/app-architecture.md` 和 `kb/docs/gpui-kit-guide.md`，见 M4a spec

### 跨期的风险

| 风险 | 对策 |
|---|---|
| `list` 只测量看得见的项，视口外是估算高度，拖动滚动条时滑块会跳 | 先接受；明显时再给 `reset_with_uniform_height` 一个平均高度。不用 `measure_all`（要布局全部行） |
| 记录文件的格式每个月都在变 | 新字段一律 `Option`，不认识的 block 和记录类型忽略 |
| 改 `parse.rs` 影响索引和搜索 | `RawBlock` 只加 `Option` 字段；改完跑 `make parity` |
| 画稿是私有 Artifact，换账号看不到 | 尺寸和色值在 spec 里写了关键的几项；其余实现时用 Artifact 工具读取 `project/*.dc.html` |
