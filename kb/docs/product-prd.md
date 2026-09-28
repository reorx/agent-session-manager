---
created: 2026-09-15
updated: 2026-09-28
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

截至 2026-09-21 完成 M0（技术栈验证）、界面重构（见 `../notes/2026-09-16-design-refactor.md`）、M1（核心数据层与 `asm` 命令行，见 `../notes/2026-09-17-m1-core-crate.md`）M2（界面接入真实数据，见 `../notes/2026-09-21-m2-real-data.md`）和 M3（界面搜索，见 `../notes/2026-09-21-m3-search.md`）。**App 现在显示本机真实的 session：启动即读索引出表，后台增量扫描，首次启动边索引边填充。** 搜索框已接上全文与正则搜索。下表用三种状态标记各模块：

| 状态 | 含义 |
|---|---|
| ✅ 已实现 | 在真实数据上可用 |
| 🟡 界面已就位 | 控件已显示，但操作不影响结果 |
| ⬜ 未开始 | 仅有需求定义 |

| 模块 | 状态 | 说明 |
|---|---|---|
| Session 表格 | ✅ | 7 列（Project / Preview / Started / Updated / Prompts / Size / Kind）、36px 两行、虚拟滚动、行选中、表头排序；Preview 列随窗口自适应 |
| Preview 展示模式切换 | ✅ | 工具栏 Preview 下拉或 `cmd-1..4`，四种模式即时切换 |
| 详情面板 | ✅ Metadata / 🟡 Conversation | 头部 + 两个 tab；Metadata 为真实数据（含 cost、时长、模型、权限模式、top 工具）；Conversation 暂时显示索引里的 user / assistant 文本（前 300 条），工具调用 / thinking 的折叠行要等 M4 直接读 transcript；尚无 Markdown 渲染。M4 方案已写好（`../plans/2026-09-28-m4-detail-panel.md`），尚未开工 |
| 分栏与折叠 | ✅ | 拖拽调宽，快捷键折叠 |
| 状态栏 | ✅ | 左：显示数 / 总数；中：操作状态 + 索引状态（loading / indexing n / m / 完成摘要 / 错误）；右：消息数、DB 大小、首批行上屏耗时 |
| 搜索框与范围切换 | ✅ | 搜索框（`cmd-f` 聚焦、Enter 回到表格、`esc` 清空）、Scope 多选、`.*` 正则开关、`Aa` 大小写、Labels 多选；选项重启后保留 |
| 时间范围过滤 | ✅ 预设 / ⬜ 自定义区间 | Updated 下拉：Today / 3 / 7 / 30 days / Any time，即时过滤 |
| 项目过滤 | ✅ 单选 + 隐藏项目 | Project 下拉按最近活动列出真实项目与 session 数；可把噪音项目（如 CodexBar 的 `ClaudeProbe`）整个隐藏并随时恢复；多选、可搜索、worktree 聚合待做 |
| Kind 过滤 | ✅ | Kind 多选下拉（CLI / SDK / BG / History，各带 session 数），默认全选，选择持久化 |
| 更多过滤 | ⬜ | 最少 prompt 数的界面控件、使用过的工具、有 PR、有 subagent |
| 全文搜索、正则搜索 | ✅ | 文本模式 FTS5（2–40ms，两字 CJK 走 LIKE 约 100ms）、正则模式流式扫描 transcript（热缓存 240–425ms）；Matches 列、高亮片段、详情内命中跳转。`asm search` 命令行同样可用 |
| 真实数据读取与索引 | ✅ | 解析器、增量 SQLite 索引、history.jsonl 补位、损坏自动重建；界面热启动约 200ms 出表（5488 行），冷索引 1.27GB 约 4s 且表格流式填充；Index 菜单可手动 Scan / Rebuild |
| 实时更新 | ⬜ | |
| 快捷键与菜单 | ✅ 部分 | 退出、折叠详情面板、`cmd-f`、`cmd-1..4`、`cmd-shift-r` 重建索引；View 菜单可显示 / 隐藏没有 prompt 的空 session |
| 设置与状态持久化 | ✅ 视图状态 / ⬜ 设置页 | 项目与时间过滤、排序、Preview 模式、空 session 开关、详情面板开合、选中行存 `state.json`，下次启动恢复 |

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
| Size | 记录文件大小 | 是 |
| Kind | 来源徽章：普通 CLI 不标记；**SDK** / **BG**（后台）/ **HIST**（记录已被清理，仅剩历史 prompt） | 是 |

Cost、Branch、Tools、ID 不再作为列，只在详情面板的 Metadata 里显示（设计稿把表格收敛到 7 列）。Preview 列第一行是标题，第二行按展示模式显示灰色预览（换行显示为 `⏎`，截到 240 字）。没有任何 prompt 的空 session 默认隐藏，View 菜单可切换。

**交互**：单击选中并刷新详情面板；双击展开详情面板；表头点击三态排序（升 / 降 / 恢复默认），同值按更新时间倒序保持稳定；列宽可拖拽；键盘上下、Home / End、翻页导航。

**待补充**（⬜）：右键菜单（复制 ID、复制 resume 命令、在 Finder 显示、打开项目目录）、Updated 一分钟内显示"live"绿点、hover 显示完整路径与 top 工具名、列显隐与列宽持久化。

### 4.2 Preview 展示模式 ✅

工具栏右侧「Preview ▾」下拉（`cmd-1..4`），决定 Preview 列第二行显示什么：**Title** / **First prompt**（默认）/ **Last prompt** / **Last message**（最后一条助手回复）。切换即时生效，不重新查询。

标题的取值优先级：用户自定义标题 > agent 名称 > AI 生成标题 > 首条 prompt 首行。

待补充（⬜）：行密度（单行 / 两行）、相对时间显示、列顺序拖拽，以上均需持久化。

### 4.3 过滤 ✅ 项目单选 + 隐藏项目 + 时间预设 + Kind 多选 / ⬜ 其余

工具栏左侧，多个条件叠加生效，结果实时更新（真实 5488 行 0–1ms，状态栏显示耗时）。过滤后保持原选中行，若它被滤掉则选中第一行。

- **时间范围**：「Updated ▾」下拉，预设 Today / 3 days / 7 days / 30 days / Any time，已过滤数据。自定义日期区间、切换为 Started 待做。
- **项目**：「Project ▾」下拉，按最近活动列出所有项目与各自 session 数，单选。待做：可搜索（真实数据上百个项目，目前靠滚动）、多选、worktree 聚合。
- **隐藏项目**（✅）：把整个项目从表格里排除，用于探针 / 监控工具产生的噪音目录（真实数据里 CodexBar 的 `ClaudeProbe` 一个目录就有 2571 条 History，隐藏后 5484 行降到 2913 行）。两个入口：选中某项目后 Project 下拉顶部出现 `Hide “<项目>”`（隐藏后项目筛选自动回到 All）；或选中一行后用 View 菜单的 "Hide Project of Selected Session"——噪音项目往往不活跃、在下拉里排得很靠后（ClaudeProbe 排第 82），从表格行直接隐藏更快。已隐藏的项目列在 Project 下拉顶部的 Hidden 区，点一下恢复；按钮显示 `All · 1 hidden` 提示有行被排除。隐藏是全局排除（与"显示空 session"同级）：Project 总数与 Kind 计数都不含隐藏项目。持久化到 `state.json`。
- **Kind**（✅）：「Kind ▾」下拉，位于 Updated 之后，多选 CLI / SDK / BG / History，每项右侧是该 Kind 的 session 数（口径与 Project 下拉一致：遵循"显示空 session"开关，不受项目 / 时间过滤影响）。按钮显示 `All`、选中项短名（如 `CLI, SDK, BG`）或 `None`（表格为空）。默认全选，不隐藏任何来源；真实数据里 History 占 83%，取消它后 5479 行降到 932 行。
- **更多过滤**（⬜）：折叠面板，含最少 prompt 数（默认 ≥1 以过滤空 session，目前由 View 菜单开关控制）、使用过的工具、有 PR、有 subagent。
- 条件持久化到下次启动（✅）；生效条件以 chip 展示可逐个移除、"Reset" 清空（⬜）。

### 4.4 搜索 ✅

- 工具栏搜索框，`cmd-f` 聚焦，输入停顿 150ms 后搜索；Enter 立即搜索并把焦点交回表格，`esc` 清空并回到表格。
- **范围**：「Scope ▾」多选下拉，User / Assistant / Tools 三个复选项，默认 User + Assistant（显示为 Both）。Tools 指工具调用的输入与输出，仅正则模式可用。
- **模式**：文本（默认，不区分大小写，多词为 AND）/ 正则（`.*` 按钮或 ⌥⌘R；非法正则按字面量匹配）。正则模式下多出 `Aa`（区分大小写）和「Labels ▾」标签过滤，如只搜 Bash 调用或 thinking 内容。
- **结果呈现**：表格只保留有命中的 session，新增 Matches 列显示命中数并可排序；Preview 列改为显示第一个命中片段并高亮；详情面板内高亮命中，`▲ 2 / 9 hits ▼`、`n` / `shift-n`（表格有焦点时）或 `cmd-g` / `cmd-shift-g` 逐个跳转。工具调用与 thinking 里的命中目前只在表格片段里可见，详情面板要等 4.5 直接读 transcript。
- 搜索与过滤条件叠加（只在通过过滤的 session 里搜）；结果上限为最新的 200 个 session，超出时状态栏提示；命中数与耗时显示在状态栏。

### 4.5 详情面板 ✅ Metadata / 🟡 Conversation

右侧分栏，默认 580px，可拖拽，`cmd-\` 折叠；未选中时显示"Select a session"。

**头部**：标题、`id · project · branch · Kind 徽章`、Metadata / Conversation 分段 tab、右侧 `N prompts · N tools · size`。

**Metadata tab**：Project、Branch、Path、ID、Started、Updated、Prompts、Tools、Size、Cost、Messages、Duration、Kind、Subagents、权限模式、Claude Code 版本、Models、Top tools、PR、Resume 命令，以及 First prompt / Last prompt / Last message 全文（各截到 2000 字）。

**当前的 Conversation**（M2 过渡实现）：显示索引里该 session 的 user / assistant 文本，最多 300 条、每条 4000 字，跳过 harness 注入的消息；底部说明未显示的内容。下面描述的工具调用 / thinking 折叠行界面已就位，M4 直接读 transcript 后启用。

**Conversation tab**：顶部 User / Assistant / Tools / Thinking 四个带计数的 chip 可切换显示，右侧 Expand all / Collapse all。user 消息浅蓝底带 `USER #n` 与时间；assistant 消息 `ASST`；thinking 与工具调用默认收成 22px 单行（时间 · ▶ · 工具名 · 参数 · 结果摘要），点击展开 input / output 或 thinking 全文。History 类 session 只显示 user prompt。

**待补充**（M4，尚未开工；实现方案见 `../plans/2026-09-28-m4-detail-panel.md`，其中 6 项选择还在等确认）：

第一段，决定"打开速度"这项指标：

- 消息流直接读记录文件，显示全部消息，不再有 300 条和 4000 字的上限
- 工具调用与 thinking 的折叠行有真实内容；一次工具调用和它的返回结果合成一张卡片
- 搜索命中落在工具调用或 thinking 里时，详情面板里同样高亮并可跳转
- 索引扫描进行中选中一行，对话照常显示

第二段：

- 头部增加 token 用量、修改过的文件列表（折叠）
- 操作按钮：复制 session ID、复制 `claude --resume <id>`、在 Finder 显示记录文件、打开项目目录、导出 Markdown。"在终端打开"方案里默认不做，只复制命令
- 助手正文 Markdown 渲染，长消息默认折叠 5 行。搜索时有命中的消息会退回带高亮的纯文本显示
- 工具卡片展开后显示说明（`#`）与命令（`$`），长命令重新排版；连续工具调用分组（设计稿 `groupTools`）需先核对设计稿
- subagent 列表，点击切换查看其消息流
- History 类 session 显示 history.jsonl 里的 prompt 列表，并提示记录已被清理

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

22px 等宽字体。左：当前显示 N / 总 M 个 session；中：最近一次操作状态（过滤耗时等）+ 索引状态（`loading index…` / `indexing 598 / 941 files` / `index up to date · 941 files checked in 0.07s` / 错误信息）；右：已索引消息数、DB 大小、首批行上屏耗时。搜索时中间显示「N sessions · M matches」，右侧最前面是搜索耗时。待补充：生效过滤 chip。

### 4.9 设置 ⬜

数据目录（默认 `~/.claude`，可改以支持多账号）、主题（跟随系统 / 亮 / 暗）、终端应用、默认过滤、是否索引 subagent。所有视图状态（排序、列宽、展示模式、过滤条件、窗口尺寸）自动持久化。

## 5. 核心使用流程

1. **启动**：热启动直接显示表格；首次启动看到索引进度，行逐步出现。
2. **缩小范围**：选项目、选时间段，表格即时收窄；状态栏显示 N / M。
3. **找到目标**：按列排序，或输入关键词搜索，Preview 列显示命中片段。
4. **查看**：单击行，右侧看摘要与消息流；搜索时直接定位到命中处。
5. **回到对话**：复制 resume 命令或直接在终端打开，继续这个 session。

## 6. 性能目标

| 指标 | 目标 |
|---|---|
| 热启动到表格可交互 | < 300ms |
| 首次全量索引（约 1GB） | < 3s，期间界面可用 |
| 过滤 / 排序 | < 16ms（M0 实测 10k 行排序 7ms） |
| 全文搜索 | < 100ms；正则 < 500ms |
| 打开 30MB 的 session 详情 | < 200ms。本机最大的记录文件已到 70MB，M4 方案建议把口径改为"选中行到对话首帧上屏，以最大的记录文件为基准"，待确认 |
| 常驻内存 | < 150MB |

## 7. 术语

- **Session**：一次 Claude Code 对话，对应一个 JSONL 记录文件；ID 为 uuid。
- **Kind**：session 来源。CLI 为终端交互；SDK 为通过 Agent SDK 发起；BG 为后台运行；History 表示记录文件已被自动清理，仅剩 history.jsonl 里的 prompt。
- **Subagent**：主 session 派生的子任务对话，存于主 session 的 `subagents/` 目录下。
- **Prompt**：用户发出的一条消息；Prompts 列即用户消息数。

## 8. 相关文档

- 原始 PRD（含实现方案与里程碑）：`../plans/2026-09-08-agent-session-manager-prd.md`
- M4 详情面板实现方案（含待确认项）：`../plans/2026-09-28-m4-detail-panel.md`
- M0 验证记录与截图说明：`../notes/2026-09-15-m0-spike.md`；界面重构记录（设计稿、模块划分、gpui-kit 坑、与设计稿差异）：`../notes/2026-09-16-design-refactor.md`；M1 核心库记录（模块、实测、与旧脚本的比对）：`../notes/2026-09-17-m1-core-crate.md`
- 数据格式：`claude-session-data-format.md`；数据层设计：`data-layer-design.md`；UI 框架：`gpui-framework-research.md`
