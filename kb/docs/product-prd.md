---
created: 2026-09-15
updated: 2026-10-27
tags:
  - prd
  - product
  - agent-session-manager
---

# Agent Session Manager 产品 PRD

面向产品视角的需求文档：说明这个应用是什么、由哪几块组成、每块怎么交互，以及各功能当前的实现状态。技术方案、数据格式、里程碑排期见文末"相关文档"，本文不重复。

## 1. 产品定位

**一句话**：一个 macOS 桌面应用，把本机所有 Claude Code 对话（session）汇总成一张可筛选、可排序、可全文搜索的表格，选中即看详情，随时复制命令恢复对话。

**用户**：使用 Claude Code 的开发者本人，单机使用，本机数据量约 85 个项目、几百个 session、近 1GB 记录文件。

**要解决的痛点**

- 记录散落在 `~/.claude` 下几百个 JSONL 文件里，找"上周在哪个项目里讨论过某个问题"只能靠命令行脚本全量扫描，慢且不能交互
- 脚本只能看到 session 摘要，看不到消息全文，跨项目切换要反复跑命令
- 被 Claude Code 自动清理掉的旧 session 只剩零散的 prompt 历史，难以追溯

**核心价值**：快。热启动即出表格，筛选排序在一帧内完成，全文搜索百毫秒级返回。

**一期非目标**：编辑或删除记录、启动新 session、云同步、其他平台打包、Codex 等其他 agent 的记录（数据模型预留扩展位）。

## 2. 实现状态

截至 2026-09-21 完成 M0（技术栈验证）、界面重构（见 `../notes/2026-09-16-design-refactor.md`）、M1（核心数据层与 `asm` 命令行，见 `../notes/2026-09-17-m1-core-crate.md`）M2（界面接入真实数据，见 `../notes/2026-09-21-m2-real-data.md`）和 M3（界面搜索，见 `../notes/2026-09-21-m3-search.md`），2026-10-27 完成 M4a（详情按回合卡显示完整对话，见 `../plans/2026-10-26-m4a-turn-cards-spec.md`）。**App 现在显示本机真实的 session：启动即读索引出表，后台增量扫描，首次启动边索引边填充。** 搜索框已接上全文与正则搜索。下表用三种状态标记各模块：

| 状态 | 含义 |
|---|---|
| ✅ 已实现 | 在真实数据上可用 |
| 🟡 界面已就位 | 控件已显示，但操作不影响结果 |
| ⬜ 未开始 | 仅有需求定义 |

| 模块 | 状态 | 说明 |
|---|---|---|
| Session 收藏 | ✅ | 2026-10-26 完成：⌘D、Favorites 菜单、行右键和详情星标；快照优先、仅收藏过滤、目录迁移、最后副本确认，见 `../plans/2026-10-26-favorites.md` |
| Session 表格 | ✅ | 9 列（Project / Preview / Started / Updated / Prompts / Cost / Ctx / Size / Kind）、36px 两行、虚拟滚动、行选中、表头排序；Preview 列随窗口自适应 |
| 费用与 context | ✅ | 2026-10-23 完成：Cost 按 token 用量和官方价格自算（含 subagent），Ctx 是结束时的 context 占用；状态栏合计当前显示行的费用；`asm cost` 命令行同样可用，见 `../plans/2026-10-23-cost-context-columns.md` |
| Preview 展示模式切换 | ✅ | 工具栏 Preview 下拉或 `cmd-1..4`，四种模式即时切换 |
| 详情面板 | ✅ Metadata / ✅ Conversation / ⬜ 操作入口 | 头部 + 两个 tab；Metadata 为真实数据（含自算费用与 Claude Code 记录的费用、token 用量、context、时长、模型、权限模式、top 工具）；Conversation 2026-10-27 完成 M4a：直接读记录文件，按回合卡显示全部对话（工具调用、thinking、AUTO、compact、打断与 API 错误），命中在哪都能跳过去，扫描期间照常显示，70MB 的 session 首帧 45–79ms（release，页缓存热；冷的一般 150–160ms）。标题行操作按钮、右键菜单扩展、导出 TXT、History 的 prompt 列表、代码块框属于 M4b，未开工（`../plans/2026-10-26-m4-detail-panel.md`） |
| 分栏与折叠 | ✅ | 拖拽调宽，快捷键折叠 |
| 状态栏 | ✅ | 左：显示数 / 总数 · 显示行的费用合计；中：操作状态 + 索引状态（loading / indexing n / m / 完成摘要 / 错误）；右：消息数、DB 大小、首批行上屏耗时 |
| 搜索框与范围切换 | ✅ | 搜索框（`cmd-f` 聚焦、Enter 回到表格、`esc` 清空）、Scope 多选、`.*` 正则开关、`Aa` 大小写、Labels 多选；选项重启后保留 |
| 时间范围过滤 | ✅ | Updated 下拉：Today / 3 / 7 / 30 days / Any time / This month / Last month / 自定义起止日期，即时过滤 |
| 项目过滤 | ✅ 单选 + 隐藏项目 | Project 下拉按最近活动列出真实项目与 session 数；可把噪音项目（如 CodexBar 的 `ClaudeProbe`）整个隐藏并随时恢复；多选、可搜索、worktree 聚合待做 |
| Kind 过滤 | ✅ | Kind 多选下拉（CLI / SDK / BG / History，各带 session 数），默认全选，选择持久化 |
| 更多过滤 | ⬜ | 最少 prompt 数的界面控件、使用过的工具、有 PR、有 subagent |
| 全文搜索、正则搜索 | ✅ | 文本模式 FTS5（2–40ms，两字 CJK 走 LIKE 约 20ms）、正则模式流式扫描 transcript（2GB 记录文件，260–530ms，带 Tools 范围时已到 500ms 的目标线）；Matches 列、高亮片段、详情内命中跳转（命中在工具输入输出、thinking 里也能跳到）。`asm search` 命令行同样可用 |
| 真实数据读取与索引 | ✅ | 解析器、增量 SQLite 索引、history.jsonl 补位、损坏自动重建；界面热启动约 200ms 出表（5488 行），冷索引 1.27GB 约 4s 且表格流式填充；Index 菜单可手动 Scan / Rebuild |
| 实时更新 | ⬜ | |
| 快捷键与菜单 | ✅ 部分 | 退出、折叠详情面板、`cmd-f`、`cmd-1..4`、`cmd-shift-r` 重建索引；View 菜单可显示 / 隐藏没有 prompt 的空 session。启动后焦点在表格上，不需要先点击就能用快捷键 |
| 设置与状态持久化 | ✅ 视图状态 / ⬜ 设置页 | 项目与时间过滤（含自定义区间）、排序、Preview 模式、空 session 开关、详情面板开合、选中行存 `state.json`，下次启动恢复 |
| 精简 TXT 导出 | ✅ CLI / ⬜ GUI | 2026-10-10 完成 Transcript Reader：通过 Session ID、唯一前缀或文件路径导出用户输入、最终回复与工具计数，去掉思考和工具结果；GUI 待 M4b 复用 core 接口，见 `../notes/2026-10-10-transcript-reader.md` |

## 3. 产品架构

```text
数据源                     核心层                         界面层
~/.claude/projects/*.jsonl   解析 → SQLite 索引 + 全文索引   工具栏（过滤 / 搜索 / 展示模式）
~/.claude/history.jsonl      增量更新 + 文件监听              表格  |  详情面板
                             搜索（全文 / 正则）             状态栏
```

三条原则决定了产品体验：

1. **索引一次，反复查询**。首次启动做全量索引并显示进度，之后启动只做增量，热启动目标 300ms 内可交互。
2. **摘要常驻内存，正文按需读取**。表格所需的几千行摘要全部在内存里，筛选排序不碰磁盘；消息全文只在选中 session 时读取对应文件。
3. **表格与详情联动**。左侧表格是唯一的导航入口，右侧详情面板随选中行变化，不开新窗口。

界面固定为三段式布局：顶部工具栏、中间可拖拽分栏（表格 + 详情面板）、底部状态栏。窗口默认 1440×900。

## 4. 功能与交互

### 4.1 Session 表格 ✅

主视图，一行一个 session，默认按更新时间倒序。

| 列 | 内容 | 可排序 |
|---|---|---|
| Project | 项目名，取路径末段 | 是 |
| Preview | 两行：第一行标题，第二行按展示模式显示的预览文本（灰色） | 否 |
| Started / Updated | 首条消息时间 / 最后活动时间，`MM-DD HH:mm` | 是，Updated 为默认排序 |
| Prompts | 用户消息数 | 是 |
| Cost | session 整个生命周期的费用（含 subagent），如 `$3.42`；算不出来显示 `—` | 是；没有费用的视为最低，降序时排在最后 |
| Ctx | 结束时 context 占用：最后一次主线调用的输入 token（含缓存）÷ 该模型的窗口，如 `41%`、`<1%` | 是；没有数据的视为最低，降序时排在最后 |
| Size | 记录文件大小 | 是 |
| Kind | 来源徽章：普通 CLI 不标记；**SDK** / **BG**（后台）/ **HIST**（记录已被清理，仅剩历史 prompt） | 是 |

Branch、Tools、ID 不作为列，只在详情面板的 Metadata 里显示（设计稿把表格收敛到 7 列，Cost 和 Ctx 是后加的）。

**费用怎么算**：每次 API 调用的 token 用量（输入、输出、5 分钟 / 1 小时缓存写入、缓存读取、fast mode）乘以官方价格，subagent 的调用计入它所属的 session。Claude Code 自己也会记一个数（`cost-state`），但很多 session 没有这条记录或记成 0，被 resume 过的 session 往往只记了最后一次运行，所以只在详情里作为对照。本机实测：Claude Code 记了费用的 973 个 session 里 96% 与自算相差不到 10%，合计相差约 1%。自算不含记录文件之外的调用（标题生成、分类器等）。新模型上线后要更新价格表，在此之前它的调用不计价，详情里会列出这些模型。Preview 列第一行是标题，第二行按展示模式显示灰色预览（换行显示为 `⏎`，截到 240 字）。没有任何 prompt 的空 session 默认隐藏，View 菜单可切换。

**交互**：单击选中并刷新详情面板；双击展开详情面板；表头点击三态排序（升 / 降 / 恢复默认），同值按更新时间倒序保持稳定；列宽可拖拽；键盘上下、Home / End、翻页导航。

**待补充**（⬜）：右键菜单（复制 ID、复制 resume 命令、在 Finder 显示、打开项目目录）、Updated 一分钟内显示"live"绿点、hover 显示完整路径与 top 工具名、列显隐与列宽持久化。

### 4.2 Preview 展示模式 ✅

工具栏右侧「Preview ▾」下拉（`cmd-1..4`），决定 Preview 列第二行显示什么：**Title** / **First prompt**（默认）/ **Last prompt** / **Last message**（最后一条助手回复）。切换即时生效，不重新查询。

标题的取值优先级：用户自定义标题 > agent 名称 > AI 生成标题 > 首条 prompt 首行。

待补充（⬜）：行密度（单行 / 两行）、相对时间显示、列顺序拖拽，以上均需持久化。

### 4.3 过滤 ✅ 项目单选 + 隐藏项目 + 时间预设 + Kind 多选 / ⬜ 其余

工具栏左侧，多个条件叠加生效，结果实时更新（真实 5488 行 0–1ms，状态栏显示耗时）。过滤后保持原选中行，若它被滤掉则选中第一行。

- **时间范围**：「Updated ▾」下拉，预设 Today / 3 days / 7 days / 30 days / Any time、This month / Last month，分隔线后是 Custom range…。选 Custom 后按钮右侧出现日期区间选择器（两个月并排，左侧有本月 / 上月 / 近 7 天 / 近 30 天快捷项，起止日都包含在内，首次默认本月 1 日到今天）；选完表格立即过滤，焦点回到表格。判断的是 session 的最后活动时间，费用仍按整个 session 计，不按天切分。切换为 Started 待做。
- **项目**：「Project ▾」下拉，按最近活动列出所有项目与各自 session 数，单选。待做：可搜索（真实数据上百个项目，目前靠滚动）、多选、worktree 聚合。
- **隐藏项目**（✅）：把整个项目从表格里排除，用于探针 / 监控工具产生的噪音目录（真实数据里 CodexBar 的 `ClaudeProbe` 一个目录就有 2571 条 History，隐藏后 5484 行降到 2913 行）。两个入口：选中某项目后 Project 下拉顶部出现 `Hide “<项目>”`（隐藏后项目筛选自动回到 All）；或选中一行后用 View 菜单的 "Hide Project of Selected Session"——噪音项目往往不活跃、在下拉里排得很靠后（ClaudeProbe 排第 82），从表格行直接隐藏更快。已隐藏的项目列在 Project 下拉顶部的 Hidden 区，点一下恢复；按钮显示 `All · 1 hidden` 提示有行被排除。隐藏是全局排除（与"显示空 session"同级）：Project 总数与 Kind 计数都不含隐藏项目。持久化到 `state.json`。
- **Kind**（✅）：「Kind ▾」下拉，位于 Updated 之后，多选 CLI / SDK / BG / History，每项右侧是该 Kind 的 session 数（口径与 Project 下拉一致：遵循"显示空 session"开关，不受项目 / 时间过滤影响）。按钮显示 `All`、选中项短名（如 `CLI, SDK, BG`）或 `None`（表格为空）。默认全选，不隐藏任何来源；真实数据里 History 占 83%，取消它后 5479 行降到 932 行。
- **更多过滤**（⬜）：折叠面板，含最少 prompt 数（默认 ≥1 以过滤空 session，目前由 View 菜单开关控制）、使用过的工具、有 PR、有 subagent。
- 条件持久化到下次启动（✅）；生效条件以 chip 展示可逐个移除、"Reset" 清空（⬜）。

### 4.4 搜索 ✅

- 工具栏搜索框，`cmd-f` 聚焦，输入停顿 150ms 后搜索；Enter 立即搜索并把焦点交回表格，`esc` 清空并回到表格。
- **范围**：「Scope ▾」多选下拉，User / Assistant / Tools 三个复选项，默认 User + Assistant（显示为 Both）。Tools 指工具调用的输入与输出，仅正则模式可用。
- **模式**：文本（默认，不区分大小写，多词为 AND）/ 正则（`.*` 按钮或 ⌥⌘R；非法正则按字面量匹配）。正则模式下多出 `Aa`（区分大小写）和「Labels ▾」标签过滤，如只搜 Bash 调用或 thinking 内容。
- **结果呈现**：表格只保留有命中的 session，新增 Matches 列显示命中数并可排序；Preview 列改为显示第一个命中片段并高亮；详情面板内高亮命中，`▲ 2 / 9 hits ▼`、`n` / `shift-n`（表格有焦点时）或 `cmd-g` / `cmd-shift-g` 逐个跳转。命中落在工具调用的输入输出、thinking、回复或压缩摘要里时，详情面板同样高亮并能跳过去（见 4.5）；详情的命中数按"命中位置"计，和 Matches 列可能不同。
- 搜索与过滤条件叠加（只在通过过滤的 session 里搜）；结果上限为最新的 200 个 session，超出时状态栏提示；命中数与耗时显示在状态栏。

### 4.5 详情面板 ✅ Metadata / ✅ Conversation / ⬜ 操作入口

右侧分栏，默认 580px，可拖拽，`cmd-\` 折叠；未选中时显示"Select a session"。

**头部**：标题、`id · project · branch · Kind 徽章`、Metadata / Conversation 分段 tab、右侧 `N prompts · N tools · size`。

**Metadata tab**：Project、Branch、Path、ID、Started、Updated、Prompts、Tools、Size、Cost（自算，旁边是 Claude Code 记录的 `CC: $x`）、Tokens（输入 / 输出 / 缓存读 / 缓存写）、Context（最终占用 / 窗口 · 峰值）、算不了价的模型（有才显示）、Messages、Duration、Kind、Subagents、权限模式、Claude Code 版本、Models、Top tools、PR、Resume 命令，以及 First prompt / Last prompt / Last message 全文（各截到 2000 字）。

**Conversation tab**：直接读这个 session 的记录文件，按回合显示全部对话，没有条数上限。读取在后台线程进行，索引扫描进行中选中一行也照常显示。

- **顶栏**：`9 turns · 2 auto · 214 tool calls`；有搜索时多出 `▲ 3 / 9 hits ▼`；最右边 Expand all / Collapse all 展开或收起所有回合的步骤。
- **回合卡**：一个回合从用户的一条真实输入开始，标题 `TURN n`。卡头依次是状态灯（绿：正常结束；红：被打断或 API 出错；灰：没有回复）、开始时间、步数与用时、出错数和结束说明，Show steps 开关在这一行右端。prompt 最多显示 4 行、最终回复最多 6 行，超出的用 Show all 展开；折叠按原文行数算，一整段不换行的长文约 900 字后折叠。prompt 里粘贴的图片或文档显示成 `image · 231 KB` 这样的标签。
- **刻度条与统计行**：一步一格（工具调用是竖杠、出错的是红色；过程文字是实心点；thinking 是空心点），下面写各工具的次数和文字、thinking 的段数。鼠标停在一格上，统计行改写成这一步（`35 / 120  Read  …/10.jpg`）；点一格会展开这个回合、滚到这一步并展开它。
- **步骤**：展开后每步一行。工具调用是 `时间 ▶ 工具名 参数摘要 结果摘要`（`212 lines`、`ok`、红色的 `error · 46 lines`），点开看 input 和 output（output 先显示 12 行）；thinking 一行，点开看全文；Agent 在工具调用之间说的话直接显示成一段 ASST 文字。展开状态在选中期间保留，换一个 session 回到全部折叠。
- **特殊段落**：不是人发起的消息（后台任务通知、自动续跑、其他 Agent 发来的消息）显示成不编号的 AUTO 卡，带来源标签；第一条输入之前已有的 Agent 活动放进一张 "session start" AUTO 卡。上下文压缩显示为一条分隔线 `context compacted · 时间`，压缩摘要可以展开。被打断的回合底部写 `[Request interrupted by user]`，API 出错的回合底部是红框里的错误文字。
- **长文本**：prompt、回复、过程文字、thinking 各保留前 2 万字，工具输出保留前 3000 字。被截过的文本，展开链接只数保留下来的部分并注明截断（`+ 47 more lines · cut, 3,000 of 16,319 chars`）。
- **搜索命中**：选中时直接停在第一个命中位置，命中所在的回合、步骤和被折叠的段落自动展开；命中黄色、当前命中橙色；卡片标题写这张卡的命中位置数，刻度上含命中的格子带黄圈。命中落在保留范围之后时，保留的那一段移到命中附近。改搜索词会重读并保留已展开的回合；清空搜索回到顶部。
- **状态**：读取中显示几张卡片骨架；读取失败（文件刚被删、没有读权限）显示一行错误说明；一条消息都没有时显示 "No messages in this session"；记录文件已被清理的 History session 显示一句说明。
- 回复按纯文本显示，代码块的 ``` 原样出现，没有 Markdown 渲染。

**待补充**（M4b，未开工；spec 见 `../plans/2026-10-26-m4b-actions-and-extras-spec.md`）：

- 标题行操作按钮（收藏、复制 resume 命令、复制 session ID、在 Finder 显示记录文件、打开项目目录、导出 TXT），表格右键菜单同样提供，表格有焦点时 `⌘C` / `⇧⌘C` 复制 ID / resume 命令。"在终端打开"放到 M5
- Metadata 增加修改过的文件列表
- 回复里的代码块显示成等宽框；不做完整的 Markdown 渲染
- `[` / `]` 在回合之间跳转
- History 类 session 显示 history.jsonl 里的 prompt 列表，并提示记录已被清理
- subagent 查看放到 M5

### 4.6 索引与实时更新 ✅ 索引与界面 / ⬜ 监听

- 首次启动全量索引，状态栏显示进度，表格随索引流式填充
- 之后启动增量更新，通常 100ms 内完成
- 运行期间监听数据目录，新 session / 新消息自动出现，Updated 列实时刷新
- 记录文件被清理后行保留并标记为 History
- 菜单提供"Rebuild Index"；索引损坏时自动重建，不崩溃
- subagent 默认不作为独立行出现，只在主 session 的 Kind 列显示数量；设置里可切换为缩进子行

### 4.7 快捷键与菜单 ✅ 部分

已实现：`cmd-q` 退出、`cmd-\` 折叠 / 展开详情面板、`cmd-f` 聚焦搜索框、`cmd-1..4` 切换展示模式，对应 App 菜单与 View 菜单。

M2 新增：`cmd-shift-r` 重建索引，Index 菜单（Scan for Changes / Rebuild Index），View 菜单的空 session 开关。

待补充：`enter` 打开详情、`cmd-c` 复制 ID、`cmd-shift-c` 复制 resume 命令、`cmd-r` 在终端 resume、`cmd-,` 设置。

### 4.8 状态栏 ✅ 部分

22px 等宽字体。左：当前显示 N / 总 M 个 session 与这 N 个 session 的费用合计（搜索时只合计命中的）；中：最近一次操作状态（过滤耗时等）+ 索引状态（`loading index…` / `indexing 598 / 941 files` / `index up to date · 941 files checked in 0.07s` / 错误信息）；右：已索引消息数、DB 大小、首批行上屏耗时。搜索时中间显示「N sessions · M matches」，右侧最前面是搜索耗时。待补充：生效过滤 chip。

### 4.9 设置 ⬜

数据目录（默认 `~/.claude`，可改以支持多账号）、主题（跟随系统 / 亮 / 暗）、终端应用、默认过滤、是否索引 subagent。所有视图状态（排序、列宽、展示模式、过滤条件、窗口尺寸）自动持久化。

## 5. 核心使用流程

1. **启动**：热启动直接显示表格；首次启动看到索引进度，行逐步出现。
2. **缩小范围**：选项目、选时间段，表格即时收窄；状态栏显示 N / M 和这些 session 的费用合计。按 Cost 或 Ctx 排序找出最贵、context 最满的 session。
3. **找到目标**：按列排序，或输入关键词搜索，Preview 列显示命中片段。
4. **查看**：单击行，右侧看摘要与消息流；搜索时直接定位到命中处。
5. **回到对话**：复制 resume 命令或直接在终端打开，继续这个 session。

## 6. 性能目标

| 指标 | 目标 | 实测 |
|---|---|---|
| 热启动到表格可交互 | < 300ms | 2064 个 session，release 101–127ms 上屏（2026-10-27） |
| 首次全量索引（约 1GB） | < 3s，期间界面可用 | 数据已涨到 2061 个文件、3.97GB：debug 7.9s，期间表格流式填充，可选中查看对话（2026-10-27） |
| 过滤 / 排序 | < 16ms | M0：10k 行排序 7ms |
| 全文搜索 | < 100ms；正则 < 500ms | 文本 2–40ms；正则 260–530ms（2.0GB，2026-09-28） |
| 打开 session 详情 | 从选中一行到对话出现 < 200ms，基准为本机最大的记录文件（70MB），以 release 构建为准（2026-10-24 定；立项时是 30MB） | 页缓存热时 release：70MB 首帧 45–79ms（读取 20ms、转换 0.2ms），5MB 级 35–40ms。先把文件逐出页缓存再打开：release 首帧一般 152–161ms，偶发 356ms（本机文件读过一两分钟就会被逐出，冷读取是常态）（2026-10-27） |
| 常驻内存 | < 150MB | 未量 |

## 7. 术语

- **Session**：一次 Claude Code 对话，对应一个 JSONL 记录文件；ID 为 uuid。
- **Kind**：session 来源。CLI 为终端交互；SDK 为通过 Agent SDK 发起；BG 为后台运行；History 表示记录文件已被自动清理，仅剩 history.jsonl 里的 prompt。
- **Subagent**：主 session 派生的子任务对话，存于主 session 的 `subagents/` 目录下。
- **Prompt**：用户发出的一条消息；Prompts 列即用户消息数。

## 8. 相关文档

- 原始 PRD（含实现方案与里程碑）：`../plans/2026-09-08-agent-session-manager-prd.md`
- M4 详情面板总 plan（含画稿链接与两期 spec）：`../plans/2026-10-26-m4-detail-panel.md`
- M0 验证记录与截图说明：`../notes/2026-09-15-m0-spike.md`；界面重构记录（设计稿、模块划分、gpui-kit 坑、与设计稿差异）：`../notes/2026-09-16-design-refactor.md`；M1 核心库记录（模块、实测、与旧脚本的比对）：`../notes/2026-09-17-m1-core-crate.md`
- 数据格式：`claude-session-data-format.md`；数据层设计：`data-layer-design.md`；UI 框架：`gpui-framework-research.md`


## Session 收藏

已实现（2026-10-26）。通过 Favorites 菜单、⌘D、行右键或详情星标保存会话快照，Kind 下拉的 Favorites only 可与其他过滤叠加。收藏副本优先显示，原会话被清理后仍能搜索与导出。取消收藏移至废纸篓；最后副本先确认。菜单支持显示、更换或恢复默认收藏目录，更换时迁移，重复 id 保留目标副本。

验收与数据规则见 [收藏方案](../plans/2026-10-26-favorites.md)。
