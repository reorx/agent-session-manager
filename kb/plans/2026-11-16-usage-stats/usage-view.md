---
created: 2026-11-16
tags:
  - usage
  - chart
  - ui
  - gpui
  - agent-session-manager
  - plan
---

# Usage 视图：按天、按小时看费用和 token，按模型、项目、session 拆开

总 plan 是 [index.md](index.md)，共用口径在那里。这是第 3 期，依赖 [usage-ledger](usage-ledger.md)（内存里的用量数据、分日规则、自动扫描、状态栏的 Today）。界面稿：<https://claude.ai/artifact/HSkahb33aYAnvCTBpQ3Jbv>（布局以它为准，文字、分日、快捷键以本 spec 为准）。

## PRD

### 背景

应用现在只能按 session 看费用。用户想回答的是按时间的问题：今天一共送进模型多少 token、值多少钱，最近每天的用量怎么变，钱花在了哪个模型、哪个项目上。账本上线后数据已经有了，缺一个看它的界面。

### 目标

在同一个窗口里加一个 Usage 视图。选一段时间，就能看到这段时间的费用、In、Out、调用数，按小时或按天的柱状图，以及按 token 种类、模型、项目、session 的拆分。

### 功能与交互

**入口与切换**

- 工具栏最左边加一个两段的切换：`Sessions | Usage`。Sessions 是现在的会话表。
- 快捷键 `⌃1` 切到 Sessions，`⌃2` 切到 Usage（2026-11-16 确认：`⌘1`–`⌘4` 已经是 Preview 四种模式的快捷键，保持不变）。View 菜单最上面加 Sessions、Usage 两项，带这两个快捷键。
- 状态栏的 `Today $203.69` 可以点：切到 Usage，范围设为 Today。
- 菜单栏菜单加 `Open Usage`（见 [menubar-resident](menubar-resident.md)）：显示窗口并切到 Usage。
- 上次停在哪个视图，重启后还在哪个视图。
- 切到 Usage 后键盘焦点在 Usage 视图上，`⌃1` / `⌃2`、`⌘Q`、菜单都照常可用；切回 Sessions 后焦点在表格上。在 Usage 里按 `⌘F` 会切回 Sessions 并把光标放进搜索框。只对会话表有意义的快捷键和菜单项（收藏、详情面板、Preview 模式、隐藏项目）在 Usage 里不起作用。

**工具栏（Usage 模式）**

从左到右：`Sessions | Usage` 切换，时间范围，自定义范围，Project / Kind / Model 三个下拉，右侧是数据截至的时刻和一个刷新按钮。搜索框、Scope、Updated、Preview 这些会话表专用的控件在 Usage 模式下不显示。窗口窄得放不下一行时，工具栏换到第二行，数据时刻和刷新按钮留在所在那一行的最右边，不推出窗口。

- **时间范围**是一排按钮：`Today`、`Yesterday`、`7 days`、`30 days`、`This month`、`Last month`，旁边是 `Custom…`。都按 4:00 分日。`7 days` 是含今天在内的最近 7 个用量日。选 Custom 后出现日期区间选择器（和会话表 Updated 的 Custom 是同一种控件），起止日都包含在内。默认是 Today。
- **过滤**：Project、Kind、Model 和会话表共用同一份选择，在一边改了，切到另一边还在。
  - Project：列出账本里有用量的项目（包括记录已经全部删掉的项目）。
  - Kind：CLI / SDK / BG。会话表的 History 一项在这里没有意义，不显示。
  - Model：按每次调用自己的模型过滤（会话表按 session 的主模型过滤，同一个选择在两边的含义略有不同）。列出账本里出现过的模型和各自的调用数。
  - 会话表的「隐藏项目」和「不显示没有 prompt 的 session」不影响 Usage：它们的调用也花了钱，都算在内。
- **数据截至**：`as of 15:53`，是上一次扫描结束的时刻；正在扫描时是 `scanning…`。刷新按钮立即扫描一次。
- 范围、自定义的起止日、主图的指标在重启后保留。

**指标卡**

一行五张，都是所选范围、所选过滤下的数字：

| 卡 | 大数字 | 小字 |
|---|---|---|
| Cost | 费用，如 `$203.69` | 和上一段的比较，如 `yesterday by 15:00 $211.30 · ▼ 3.6%` |
| In | 送进模型的全部 token，如 `492.7M` | 三段拆分：`cache read 98.3% · cache write 8.5M · uncached 6k` |
| Out | 输出 token，如 `2.4M` | 和上一段的比较 |
| Calls | API 调用次数 | 平均每次调用的费用 |
| Sessions | 有调用的 session 数 | 涉及的项目数，如 `6 projects` |

「上一段」的定义：

| 范围 | 比较对象 |
|---|---|
| Today | 昨天到同一个钟点为止 |
| Yesterday | 前天全天 |
| 7 days / 30 days | 再往前的 7 / 30 天；最后一天只算到同一个钟点 |
| This month | 上个月的同样天数；最后一天只算到同一个钟点 |
| Last month | 再上一个月全月 |
| Custom | 紧挨在前面、同样天数的一段；范围含今天时最后一天只算到同一个钟点 |
| 某一天（从图上点进来的） | 前一天全天 |

数据按整点小时存，所以「同一个钟点」精确到小时：15:53 时，今天算到 15:53，昨天算到 16:00 之前。上一段没有任何数据时小字显示 `no earlier data`。

**主图**

- 范围是一天（Today、Yesterday、点进来的某一天，或 Custom 只选了一天）时，按小时画 24 根柱子，横轴从 04 到次日 03。每根柱子按模型堆叠。前一天同一钟点的总量用虚线框叠在后面作对比（夏令时那天按钟点对，前一天没有这个钟点就不画）。范围是今天时，在当前时刻画一条竖线，标出时刻。
- 范围多于一天时，每天一根柱子，按模型堆叠，再画一条 7 日移动平均线（当天及之前 6 天的平均）。今天那根柱子标 `so far`。
- 图上方可以切指标：`Cost`、`In`、`Out`、`Hit rate`。Hit rate（缓存读取占 In 的比例）画成折线，不堆叠，纵轴不从 0 开始。
- 悬停在一根柱子上显示明细：时间、各模型的数值、合计、In / 命中率 / Out、调用数，多天时还有 7 日均值，单天时还有前一天同一小时的值。
- 多天的图上点某一天的柱子，范围变成那一天，按小时显示；范围按钮旁出现一个带日期和 ✕ 的小标签，点 ✕ 或按 Escape 回到点进来之前的范围。没有调用的日子点了没有反应。
- 模型的颜色固定：Opus 5.5 蓝、Sonnet 5.5 橙、Fable 5.2 绿、Haiku 5.5 黄，其他所有模型合成灰色的一段 `Earlier models`。图例只列出范围内出现过的模型。颜色不是唯一的线索：图例、悬停明细和下面的表格都写着模型名。

**钱花在哪**

按 token 种类拆开的费用：一条横向的堆叠条，下面一张表，四行：Cache read、Cache write（5 分钟和 1 小时两种合并）、Output、Uncached input；列是 Tokens、Cost、Share。有 web search 的费用时加一行。

**按模型**

每个模型一行（这里不合并，`Earlier models` 里的各个模型分开列，色块是灰色）：Calls、In、Hit%、Out、Cost、Share，最后一行合计。按费用从高到低。fast 模式和长 prompt 的调用计入各自的模型，不单独成行。

**按项目**

费用最高的 7 个项目各一行：项目名、占比条、Cost、Share、Sessions（范围内有调用的 session 数）。其余合成一行 `N other projects`。点一行，Project 过滤设为这个项目（再点一次取消）；被选中的行高亮。项目名取路径的最后一段，记录全部删掉、账本里也没有路径的项目显示目录的编码名。

**费用最高的 session**

范围内费用最高的 6 个 session：短 id、标题、项目名、In、Cost。Cost 是这个 session 在所选范围内的费用，不是它的总费用。记录已被清理的 session 照常列出，标题来自账本。这一期只展示，点击跳转由 [usage-drilldown](usage-drilldown.md) 加。

**状态栏（Usage 模式）**

左侧只显示 `Today $203.69`（Sessions 模式下的 session 数和合计在这里没有意义）；中间照旧是索引状态；右侧写明口径：`by API response time · day starts 04:00`。

**刷新节奏**

- 切到 Usage 时立即扫描一次（距上次扫描结束不到 30 秒则不扫）。
- Usage 开着时每 60 秒扫描一次；切回 Sessions 后回到每 10 分钟一次。
- 扫描没有发现变化时界面不动；有变化时数字和图就地更新，不闪、不重置滚动位置。
- 时间过了 4:00，Today 等相对范围自动换到新的一天。

**空状态与异常**

- 账本里还没有任何用量（全新安装、启动扫描还没结束）：指标卡显示 `—`，图的位置写 `No usage recorded yet`；扫描结束后自动出现。
- 所选范围、过滤下没有调用：指标卡显示 `$0.00` 和 0，图是空的坐标轴，各表写 `No calls in this range`。
- 账本不可用（见 usage-ledger 的降级情形）：整个视图只显示一句说明 `Usage ledger unavailable: it was written by a newer version of the app`。
- 有调用用了价格表里没有的模型：这些调用的 token 照常计入，费用按 0 算；Cost 卡的小字后面加 `N calls unpriced`，按模型表里该模型的 Cost 显示 `—`。
- 窗口较窄时两列的区块（钱花在哪 / 按模型，按项目 / session）改成上下排列；整个视图可以纵向滚动。

### 不做的事

- 不做：星期 × 小时热力图、月底预估、换模型计价、主线与 subagent 的拆分、Prompt 数、每百万 In 的实际单价、缓存失效统计。
- 不做点击 session、项目跳到会话表（下一期）。
- 不做导出、打印、复制图表。
- 不做独立窗口，不做和会话表并排显示。
- 不做图表的缩放、框选、键盘逐柱导航。
- 不做深色主题（应用目前只有浅色）。

### 验收标准

1. `⌃2`、工具栏切换、View 菜单、点状态栏的 Today、菜单栏的 Open Usage 都能进入 Usage；`⌃1` 回到 Sessions。切换后不用点鼠标，快捷键仍然有效。`⌘1`–`⌘4` 在 Sessions 里仍然切换 Preview 模式。
2. 重启后回到上次的视图、范围和主图指标。
3. Today 的 Cost 等于状态栏的 Today；选一个项目后，Cost 等于按项目表里这个项目的费用。
4. 每个范围的起止都按 4:00 分日；`7 days` 恰好是 7 个用量日；Custom 的结束日包含在内。
5. 各范围的「上一段」按上表取，最后一天按钟点截断；上一段没有数据时显示 `no earlier data`。
6. In 等于 uncached + cache write + cache read；钱花在哪四行的费用相加等于 Cost（有 web search 时加上那一行）；按模型各行的 Cost 相加等于 Cost。
7. 单天的图是 24 根柱子（夏令时切换日除外），从 04 开始；多天的图每天一根，并有 7 日均线；四个指标都能切换。
8. 悬停显示明细；点某一天进入按小时，✕ 和 Escape 回到之前的范围。
9. Project / Kind / Model 的选择在两个视图之间保持；Usage 里 Model 按调用的模型过滤；隐藏的项目在 Usage 里照常计入。
10. 记录已被清理的 session 出现在「费用最高的 session」里，标题和项目正确。
11. Usage 开着时，新的调用在 60 秒内出现在数字里；切到 Usage 立即扫描一次；刷新按钮可用。
12. 空账本、范围内无调用、账本不可用、有未计价模型四种情况各显示上文所述的内容。
13. 表格里选了隐藏项目或 Preview 模式的快捷键、`⌘D`、`⌘\` 在 Usage 里不改变任何状态。

## Tech Design

### 现状

- usage-ledger 之后：`AppRoot` 有 `usage: UsageData`（core `usage.rs`，按 session 存小时汇总）、`today: Totals`、`usage_day`，`render_status_bar` 左侧末尾是 Today；自动扫描每 `AUTO_SCAN_IDLE`（600 秒）一次，`auto_scan` 有 30 秒的节流；`roll_day` 在换日时重算。
- `crates/app/src/main.rs`：`AppRoot::render` 是「工具栏 + `h_resizable`（表格 | 详情）+ 状态栏」。全部 action 在根元素上 `.on_action`。`bind_keys` 里 `cmd-1` 到 `cmd-4` 绑的是 `Preview1`–`Preview4`。焦点在表格（`focus_table`）或搜索框。View 菜单在 `main()` 的 `cx.set_menus` 里。
- `crates/app/src/toolbar.rs`：`render_toolbar` 画会话表的控件；`trigger(..)` 是「标签 值 ▾」按钮；`project_menu` / `kind_menu` / `model_menu` 生成下拉；`apply_filter` 把 `Filter` 交给表格并在变化时重搜。`Filter`（`model.rs`）的 `project`、`kinds`、`model` 存在表格的 delegate 上。`date_picker` 是 `DatePickerState::range`，选完后把焦点还给表格。
- `crates/app/src/settings.rs::ViewState` 是 `#[serde(default)]` 的，加字段不用迁移。
- `crates/app/src/style.rs` 集中色值；没有图表色。
- gpui-component 0.6.1（本地源码核对）：`plot::Plot` trait（`paint`、可选的 `prepaint`、`id`、`tooltip_state`、`tooltip`）和 `#[derive(IntoPlot)]`；`plot::shape::Bar`（`data`、`cross`、`band_width`、`base`、`value`、`fill`、`corner_radii`、`paint`），`base` 和 `value` 都是沿数值轴的像素位置，所以堆叠可以自己算；`plot::shape::Line`、`plot::Grid`（`dash_array`）、`plot::PlotAxis` / `AxisText`、`plot::scale::{ScaleBand, ScaleLinear}`、`plot::tooltip::{Tooltip, TooltipState, CrossLine}`。`chart::BarChart` 自带悬停提示但只有一个数值序列，没有堆叠，也没有点击回调。`button::ButtonGroup` / `ToggleGroup` 可做分段按钮。GPUI 的 `PathBuilder` 有 `dash_array`。

### 方案

core 的 `usage.rs` 扩成完整的聚合：给定小时区间和过滤，算出合计、按模型 / 项目 / session 的分组和图的各个槽位；范围、上一段、槽位的小时边界也在 core 算（依赖分日规则）。这些都是纯函数。

app 新增 `usage_view.rs`：`UsageState`（范围、指标、点进来的那一天、`FocusHandle`、自定义范围的日期选择器、一份算好的 `UsageSnapshot`）和 `AppRoot::render_usage`。`UsageSnapshot` 里全是格式化好的字符串和画图用的数值，只在用量数据、范围、过滤、用量日变化时重算，render 只取。主图是 `usage_chart.rs` 里一个实现了 `Plot` 的 `UsageChart`，用 `Bar`、`Grid`、`PlotAxis`、`Line` 拼出来。`AppRoot` 加 `view: MainView`，`render` 按它画会话表或 Usage，工具栏和状态栏各有两种形态。

关键决定：

- **Usage 是 `AppRoot` 的一部分，不是独立的实体。** 它和会话表共用过滤、store 事件、状态栏、定时器。拆成独立 `Entity` 要在两个实体之间同步这些，现阶段没有收益。代码按文件分开（`usage_view.rs`、`usage_chart.rs`），`AppRoot` 上只多一个 `usage_view: UsageState` 字段。
- **自己实现 `Plot`，不用 `BarChart`。** 需要的是堆叠、叠在后面的虚线框、均线、当前时刻线，`BarChart` 一样都没有。`Plot` 的 `tooltip_state` / `tooltip` 能复用组件库的悬停提示。
- **点击柱子在图外面处理。** `Plot` 没有点击回调。图放在一个带 id 的 `div` 里，`on_mouse_down` 时用同一个纯函数 `slot_at(x, width, slots)` 把横坐标换成槽位；这个函数也供 `tooltip_state` 和绘制使用，三处不会错位。图的绘制区域在 `paint` 时记进一个 `Rc<Cell<Bounds<Pixels>>>` 供点击换算。
- **过滤和会话表共用 `Filter` 上的三个字段。** 备选是 Usage 自己存一份，那样跳转（下一期）和「切过去不丢」都要做同步。Usage 读 `Filter.project`、`Filter.kinds` 的 cli / sdk / bg、`Filter.model`，忽略 `hidden_projects`、`show_empty`、`favorites_only`、时间范围和 `kinds.history`。
- **Usage 的 Project 和 Model 下拉用自己的条目来源**（来自 `UsageData`），菜单的画法复用 `toolbar.rs` 现有的 `menu_item`。会话表的条目来自表格的行，记录全删掉的项目不在里面。
- **视图切换的快捷键是全局绑定的 `ctrl-1` / `ctrl-2`**，在搜索框里也有效。
- **只对会话表有意义的 action 在处理函数开头判断视图**，Usage 下直接返回。备选是给会话表加一个 key context 再把这些绑定限定进去，那要改现有的全部绑定和菜单的可用性，风险更大。
- **Usage 开着时把自动扫描的间隔换成 60 秒**：usage-ledger 的定时器循环每次醒来读当前视图决定下一次睡多久；切到 Usage 时另外调一次 `auto_scan`。
- **Usage 下收到整表 `Loaded` 不重跑搜索**，记一个 `search_stale`，切回 Sessions 时再搜。Usage 开着时每 60 秒可能来一次 `Loaded`，正则搜索每次要零点几秒。
- **界面文字用英文**，和工具栏、菜单一致；界面稿上的中文标签对应到上文 PRD 里的英文。

### 数据变动

- `settings.rs::ViewState` 加：`view: MainView`（`Sessions` 默认 / `Usage`）、`usage_range: UsageRange`（默认 `Today`）、`usage_custom_range: Option<(NaiveDate, NaiveDate)>`、`usage_metric: Metric`（默认 `Cost`）。点进来的某一天不保存。
- 账本和索引不变。

### 逻辑变动

**core `usage.rs`**

- `UsageFilter { project: Option<String>, cli: bool, sdk: bool, bg: bool, model: Option<String> }`。session 的类型由 `LedgerSession` 的 `is_background` / `entrypoint` 按 `session.rs::SessionKind::of` 的后三条规则得出（抽一个不依赖 `SessionSummary` 的函数供两边用）。模型比较用 `pricing::normalize` 之后的 id。
- `UsageRange { Today, Yesterday, Days7, Days30, ThisMonth, LastMonth, Custom, Day }`（`Serialize` / `Deserialize`）；`Span { from_ms: i64, until_ms: i64 }`。
  - `range_span(range, today: NaiveDate, custom, day, tz) -> Span`：都用 `day_start_ms`；Custom 没选过时是本月 1 日到今天。
  - `previous_span(range, span, now_ms, tz) -> Option<Span>`：按 PRD 的表；需要按钟点截断时，在上一段对应的用量日上构造与现在相同的本地钟点（凌晨 0–3 点属于次日），按本地钟点向上取整到小时，再裁剪到上一段结束边界。上一月较短时先构造完整候选时刻再裁剪，不能先把日期压到月末。遇到不存在的钟点取下一有效时刻；重复钟点取较早一次，与 `day_start_ms` 一致。
- `Metric { Cost, In, Out, HitRate }`。
- `summary(&self, span, filter) -> Summary`：`Summary { totals: Totals, by_model: Vec<(String, Totals)>, by_project: Vec<ProjectUsage>, by_session: Vec<SessionTotal>, sessions: usize, projects: usize }`，各组按费用降序。`ProjectUsage { key, name, totals, sessions }`，`name` 取路径最后一段，没有路径用 key；`SessionTotal { id, title, project_name, totals }`。
- `series(&self, slots: &[Span], filter) -> Vec<SlotTotals>`：每个槽位的合计和按「颜色槽」的分组。颜色槽由 `model_slot(model) -> usize` 决定：`claude-opus-5-5` → 0，`claude-sonnet-5-5` → 1，`claude-fable-5-2` → 2，`claude-haiku-5-5` → 3，其余 → 4。
- `hour_slots(day, tz) -> Vec<Span>`（从 `day_start_ms(day)` 到下一天起点，每小时一个；夏令时那天是 23 或 25 个）；`day_slots(span, tz) -> Vec<(NaiveDate, Span)>`。
- `moving_average(values: &[f64], window: usize) -> Vec<f64>`：均线要用到范围之前 6 天的数据，调用方把槽位往前多取 6 天。
- `project_entries()` / `model_entries()`：下拉用的条目（项目：key、名字、session 数；模型：id、调用数）。
- 每个桶的费用在 `replace_all` / `apply` 时算好存在桶旁边（`Option<CostLines>`），聚合只做加法。

**app**

- `model.rs`：`MainView`；`fmt` 用的小函数（`fmt_delta(cur, prev) -> "▲ 3.6%"`、`fmt_pct`）放在这里或 core `format.rs`（已有 `fmt_usd`、`fmt_tokens`、`fmt_change`，先看能不能直接用）。
- `usage_view.rs`（新）：
  - `UsageState { range, custom, day: Option<NaiveDate>, back: UsageRange, metric, focus: FocusHandle, date_picker: Entity<DatePickerState>, snapshot: UsageSnapshot, hover: Option<usize> }`。
  - `UsageSnapshot`：五张卡的字符串、`chart: ChartData`（槽位标签、每槽各颜色槽的数值、虚线框的数值、均线、当前时刻线的位置、纵轴刻度及其标签、图例）、三张表和 session 列表的行（全是 `SharedString`）、空状态的种类。`UsageSnapshot::build(&UsageData, &UsageFilter, range, custom, day, metric, now_ms, usage_day)` 是纯函数，app 的测试直接断言它的字符串。
  - `AppRoot::refresh_usage(cx)`：重算快照并 `notify`。在这些地方调用：`on_store_events` 收到 `Usage` 之后、`apply_filter` 之后、范围 / 指标 / 点选变化之后、`roll_day` 之后、切到 Usage 时。
  - `AppRoot::render_usage(cx) -> Div`：`div().track_focus(&focus).key_context("Usage")`，里面是可纵向滚动的容器（`overflow_y_scrollbar()`）：指标卡一行、主图区、两组 `grid2`。窗口宽度小于阈值时 `grid2` 改成单列。
  - `set_usage_range`、`set_usage_metric`、`drill_into_day`、`leave_day`（Escape 和 ✕）。
- `usage_chart.rs`（新）：`#[derive(IntoPlot)] struct UsageChart { data: ChartData, bounds: Rc<Cell<Bounds<Pixels>>> }`，实现 `Plot`：`paint` 画 `Grid`、纵轴和横轴的 `PlotAxis`、虚线框（`PathBuilder` 加 `dash_array`）、各颜色槽的 `Bar`（`base` 是下面各段累加后的像素位置）、均线（`Line`）或命中率折线、当前时刻线；`id` 返回固定 id 以启用提示；`tooltip_state` 用 `slot_at`；`tooltip` 用 `plot::tooltip::Tooltip` 的 `title` 和 `row`。几何（边距、槽宽、`slot_at`、数值到像素）是文件里的纯函数。
- `toolbar.rs`：`render_toolbar` 开头画分段切换（`ButtonGroup` 或两个自定义外观的 `Button`，用 `style::segment_bg()`）；`view == Usage` 时后面画范围按钮、点选的小标签、`Custom…` 与 `DatePicker`、三个下拉、`as of`、刷新按钮，否则是现有的控件。`usage_project_menu`、`usage_kind_menu`、`usage_model_menu` 复用 `menu_item`。
- `main.rs`：
  - action `ShowSessions`、`ShowUsage`，`KeyBinding::new("ctrl-1", ShowSessions, None)`、`("ctrl-2", ShowUsage, None)`；View 菜单最上面加两项和一条分隔线。
  - `AppRoot` 加 `view: MainView`、`usage_view: UsageState`、`search_stale: bool`。
  - `set_view(view, window, cx)`：改 `view`；Usage → `refresh_usage`、焦点给 `usage_view.focus`、`auto_scan`；Sessions → `focus_table`，`search_stale` 时 `run_search`；`persist`。
  - `render`：中间部分按 `view` 画 `h_resizable(..)` 或 `render_usage`。
  - `FocusSearch` 的处理：Usage 下先 `set_view(Sessions)` 再 `focus_search`。`Escape` 的处理：Usage 下调 `leave_day`，否则照旧 `clear_search`。`ToggleDetail`、`ToggleFavorite`、`Preview1`–`4`、`HideProject`、`ToggleEmptySessions`、`NextHit` / `PrevHit` 的处理函数在 Usage 下直接返回。
  - `render_status_bar`：Usage 下左侧只有 Today，右侧是口径说明。Today 那一段包成可点击的元素（两个视图下都可点），点击调 `set_usage_range(Today)` 和 `set_view(Usage)`。
  - 自动扫描的循环：睡的时长取 `if view == Usage && !window_hidden { 60 秒 } else { 600 秒 }`（`window_hidden` 来自 menubar-resident；那一期还没合并时没有这个条件）。
  - `on_store_events`：`reloaded && searching()` 时，Usage 下只置 `search_stale`。
  - `view_state` / `AppRoot::new`：存取新字段。
  - `on_resident_command`：`OpenUsage` → `show_window` 后 `set_view(Usage)`；`OpenSessions` → `set_view(Sessions)`。`resident.rs` 的菜单加 `Open Usage` 一项。
  - `ASM_USAGE_AUTOPLAY`：见测试一节。
- `style.rs`：`model_color(slot: usize) -> Hsla`（`#2a78d6`、`#eb6834`、`#1baf7a`、`#eda100`、`#a3a3ab`）、`token_cache_read()` `#6250d6`、`token_cache_write()` `#e87ba4`、`token_output()` `#008300`、`token_input()` `#b8b8be`、`chart_ghost()` `#a9a9b0`、`chart_now()`（用现有的 `accent()`）、`delta_up()` / `delta_down()`（先看 `cost_up()` / `cost_down()` 能否复用）。这些颜色在界面稿里用 dataviz 的校验器检查过，相邻色的对比度有 WARN，所以图例和表格必须保留。
- `scripts/screenshot-usage.sh` 和 `make screenshot-usage`。

### 实施步骤

- [x] 1. core `usage.rs`：范围、上一段、槽位、`summary`、`series`、`moving_average`、条目。先写 `crates/core/tests/usage.rs` 里的用例，再实现
- [x] 2. `settings.rs` 的新字段和往返测试
- [x] 3. `UsageSnapshot::build` 的测试与实现
- [x] 4. `keyboard_tests.rs` 的视图切换与焦点用例；`MainView`、`set_view`、绑定、菜单、会话表专用 action 的判断
- [x] 5. 工具栏的两种形态、范围按钮、三个下拉、Custom
- [x] 6. `render_usage`：指标卡、三张表、session 列表、空状态
- [x] 7. `usage_chart.rs`：先写几何的测试，再实现绘制、悬停、点击
- [x] 8. 状态栏、60 秒的间隔、`search_stale`、`Open Usage`
- [x] 9. `ASM_USAGE_AUTOPLAY`、截图脚本；用 `agent-inspect-session.sh` 看 context 占用后做实机验收；对照界面稿检查布局
- [x] 10. 文档（`gpui-kit-guide.md` 加图表一节、`app-architecture.md` 的焦点表和保存表、`product-prd.md`、`AGENTS.md`）、commit

### 测试与验证

core `crates/core/tests/usage.rs`（时区用 `FixedOffset` +8，除非注明）。括号里是验收标准的编号。

- `every_range_starts_and_ends_at_four`（4）：各 `UsageRange` 的 `range_span`；`seven_days_are_seven_usage_days_including_today`；`a_custom_range_includes_its_last_day`；`before_four_today_is_the_day_before`。
- `the_previous_span_follows_the_range_and_stops_at_the_same_hour`（5）：逐个范围断言；`a_range_ending_in_the_past_compares_whole_days`。
- `a_day_has_24_hour_slots_starting_at_four`、`a_dst_day_has_23_or_25_slots`（7，`chrono_tz`）。
- `the_summary_adds_up`（3、6）：`by_model`、`by_project` 各自的费用之和等于 `totals`；`tokens.input_total()` 等于三段之和；`cost.total()` 等于各行之和。
- `filters_narrow_by_project_kind_and_response_model`（9）：模型过滤按桶的模型，不按 session 的主模型；`a_session_whose_transcript_is_gone_still_counts_and_keeps_its_title`（10）。
- `sessions_are_ranked_by_their_cost_inside_the_range`：一个跨两天的 session，只选第二天时它的费用只含第二天。
- `series_stack_models_into_fixed_slots`（7）：`model_slot` 的五种情况；每槽各颜色槽之和等于槽的合计。
- `the_moving_average_uses_up_to_six_days_before`。
- `an_unpriced_model_is_counted_in_tokens_and_reported`（12）。

app：

- `usage_view.rs`（`UsageSnapshot::build`，纯函数）：`cards_show_cost_tokens_calls_and_sessions`、`the_comparison_reads_no_earlier_data_without_a_previous_span`（5）、`an_empty_ledger_and_an_empty_range_read_differently`（12）、`unpriced_calls_are_called_out_on_the_cost_card`（12）、`the_project_table_folds_the_tail_into_other_projects`、`a_single_day_charts_hours_and_several_days_chart_days`（7）。
- `usage_chart.rs`：`slot_at_maps_the_cursor_to_the_bar_under_it`（8）、`stacked_segments_start_where_the_one_below_ends`。
- `settings.rs`：新字段往返；旧文件缺字段时是 Sessions / Today / Cost（2）。
- `keyboard_tests.rs`：
  - `control_2_shows_usage_and_control_1_goes_back_with_the_focus_following`（1）：`press("ctrl-2")` 后 `view == Usage`，`usage_view.focus` 有焦点；再 `press("ctrl-1")`，表格有焦点，`press("n")` 之类的表格快捷键可用。
  - `the_view_shortcuts_work_from_the_search_box`（1）：`cmd-f` 后 `ctrl-2`。
  - `command_1_to_4_still_switch_the_preview_mode`（1）。
  - `command_f_in_usage_goes_to_sessions_and_focuses_the_search`（1）。
  - `session_only_shortcuts_do_nothing_in_usage`（13）：`cmd-d`、`cmd-\`、`cmd-1` 之后表格的模式、详情是否可见、收藏都没变。
  - `escape_leaves_a_drilled_day`（8）：调 `drill_into_day` 后 `press("escape")`，范围回到之前的。
  - `a_filter_set_in_usage_is_still_set_in_sessions`（9）。
  - `picking_a_custom_range_in_usage_hands_the_focus_back_to_usage`（1）：照 `a_picked_custom_range_filters_the_table_and_hands_the_focus_back_to_it` 写。
  - `usage_scans_every_minute_and_sessions_every_ten`（11）：Usage 下推进时钟 60 秒有一次扫描；切回后 60 秒没有、600 秒有。
  - `a_reload_in_usage_defers_the_search_until_sessions_is_shown`。

实机验收（`ASM_DB`、`ASM_STATE` 指到 `tmp/2026-11-12-usage-stats/accept/`；数据用真实数据目录，账本由第一次启动回填；按 `~/.claude/docs/DESKTOP_APP_TESTING.md`）：

- `ASM_USAGE_AUTOPLAY=1`：启动扫描结束后逐步执行并打 `autoplay-usage: <step>`：`today` → `yesterday` → `d30` → `d30-in`（指标切到 In）→ `d30-hit` → `drill`（点进费用最高的一天）→ `back` → `project`（选费用最高的项目）→ `month` → `sessions`（切回会话表，看过滤还在）。`scripts/screenshot-usage.sh` 等标记后按窗口 ID 截图，输出到 `tmp/2026-11-12-usage-stats/screenshots/`。
- 每张图用 `Read` 打开核对：布局对照界面稿；Today 的 Cost 等于状态栏；用 `sqlite3` 对账本副本算 30 天的合计和某一天的小时分布，与图上的数字比对（验收 3、6、7）。
- 悬停提示无法程序化触发：autoplay 的 `drill` 之前加一步把 `usage_view.hover` 设到某个槽位再截图。
- 验收 11：Usage 开着，在另一个终端跑一次 Claude Code，一分钟内数字变化。
- 把窗口拉窄到 900 宽截一张，确认两列改成单列、没有横向溢出。

### 风险与待定

- **自定义 `Plot` 的工作量和细节（推测）。** `Bar` 的 `base` / `value` 按像素给，圆角只能整根设；界面稿里只有最上面一段是圆角，实现时先接受整段直角或只给最上面一段设 `corner_radii`。`Tooltip::row` 能否放色块、多行明细能否排下，以本地源码为准，不行就用 `Plot::tooltip` 返回自己拼的元素。
- **`#[derive(IntoPlot)]` 要求调用方直接依赖 `gpui-kit` 或 `gpui-pre`**（宏里的报错信息这么写）。app 依赖的是 `gpui-kit`，应当可用；编不过时手写 `IntoElement` / `Element`。
- **点击换算依赖记下来的绘制区域**：窗口尺寸变化的那一帧可能用到上一帧的区域，点偏一个槽位的概率很低，可以接受。
- **半小时时区**：`hour_slots` 的边界不在桶的边界上，按总 plan「风险与待定」处理，不做修正。
- **Model 过滤在两个视图含义不同**：Usage 按调用的模型，会话表按主模型。选了 Haiku 5.5 后切到会话表，可能只剩很少的 session。先按界面稿做共用，用起来别扭再拆成两份。
- **窗口隐藏时不要每 60 秒扫描**：已经写进间隔的条件里；menubar-resident 没合并时这个条件不存在。
- **按模型表的行数**：本机出现过 10 个模型，表格不设上限；以后模型多了再考虑折叠。

### Core 实现与实测（2026-11-16）

`usage.rs` 已提供上述范围、比较、槽位、过滤、汇总、序列和条目接口；具体签名见 [P3 core 交接](../../sessions/2026-11-16-usage-p3-core.md)。上一月天数不足时比较截止到上一月末；逆序 Custom 日期自动交换。空数据返回零汇总和零槽位，是否显示 `no earlier data` 由调用方检查比较汇总的 responses。

实现取舍：保留公开可变的 `UsageData.sessions`，不另存费用缓存，避免现有调用方直接修改数据时缓存过期。`summary_with` / `series_with` 可注入价格表，默认入口使用当前价格；旧 API 不变。将来确有性能压力时应先封装 sessions 再引入缓存。

真实账本只读 backup 副本（3,361 sessions、5,510 桶），release 模式连续 1,000 次「30 天 summary + 30 个日槽 series」平均 **2.512 ms**（不含加载，包含定价、分组和排序）。证据：`tmp/2026-11-12-usage-stats/usage-p3-core/bench.log`。目前无需为费用缓存破坏公开 API。

### App 实现与差异（usage-p3-view，2026-11-16）

实施步骤 2–6、8 已实现（图表 7 和 autoplay / 截图脚本 9 未做）。交接见 [P3 view 总结](../../sessions/2026-11-16-usage-p3-view.md)。与上文不同或上文没写的地方：

- **`UsageSnapshot::build` 的参数**：`build(&UsageData, available: Option<bool>, &UsageQuery, now_ms, today, &tz)`，`UsageQuery` 收拢了 range、custom、day、metric 和 `UsageFilter`。多一个 `available`，因为「账本不可用」和「账本还没到」都要从快照里分出来。时区作参数是为了测试用 +8。
- **`search_stale` 是一个枚举不是 bool**（`StaleSearch::Refresh` / `Rerun`）：扫描后的重读只要静默地换结果（`refresh_search`，不滚回顶部），在 Usage 里改过滤才要当新搜索跑（`run_search`）；两种都发生时取后者。
- **`refresh_usage` 在会话表显示时直接返回**，`set_view(Usage)` 时重算。快照内容和上一次相同就不 `notify`。真实账本副本上重算 1.5–4.5ms（debug 构建，日志 `usage view: … built in …`）。
- **Finder 再次打开不再发 `OpenSessions`**：`OpenSessions` 现在会切回会话表，所以加了 `ResidentCommand::Reopen`（不是菜单项），只显示窗口、保持关窗前的视图。菜单项的序号随之变成 Open Sessions 4、Open Usage 5、Scan Now 6、Quit 8。
- **扫描节奏随窗口**：窗口隐藏和重新显示时也换一个新的计时任务，所以停在 Usage 时关窗，下一次定时扫描在 10 分钟后（不是 1 分钟后）；重开后回到每分钟。
- **会话表专用的动作**除了上文列的，`⌥⌘R` / `⌥⌘C`（正则、大小写）和 `[` / `]`（回合跳转）在 Usage 下也直接返回。
- **Escape**：Usage 视图有 key context `Usage`，`escape` 在里面绑到 `LeaveDay`；根上的 `Escape` 处理在 Usage 下也调 `leave_day`（搜索框里的 Escape 是输入框自己的 action，到不了这里）。
- **点选某一天**：`drill_into_day` 返回 bool，在按小时的图上或那天没有调用时返回 false、什么都不改。保存视图状态时 `Day` 存成点之前的范围；万一读到 `Day`，按 Today 打开。
- **Kind 下拉**只有 CLI / SDK / BG，后面的数字是账本里有调用的 session 数；**Project 下拉**列账本里所有有调用的项目（按名字排序，包括会话表里隐藏的），选中的项目在账本里没有调用时也列在最上面，好取消；**Model 下拉**按调用数从多到少，值用 `display_name`。
- **窄窗口的阈值是 960pt**（窗口内容宽度），小于它时两组 `grid2` 改成上下排列。
- **Cost 卡的数字用两位小数加千分位**（`$8,010.62`，`usage_view::fmt_money`），没有用 core 的 `fmt_usd`（它对大数省掉小数）；In / Out 用 `fmt_tokens`。比较的说法：Today `yesterday by HH:MM`、Yesterday 和点进的那一天 `day before`、`prior 7 days` / `prior 30 days` / `prior N days`（Custom）、This month `same days last month`、Last month `month before`。上一段没有调用时整段小字是 `no earlier data`。
- **范围内没有调用时**「钱花在哪」只画表头和 `No calls in this range`，不列四行 0。
- **刷新按钮**在扫描进行中置灰；`as of HH:MM` 取最近一次扫描结束的时刻，扫描中是 `scanning…`。
- **图表区**：`render_chart_panel` 画标题、副标题、指标切换和图例，下面是 `id("usage-chart")`、高 236 的空容器（`usage_view::CHART_ID` / `CHART_H`），账本为空时里面写 `No usage recorded yet`。副标题：单天是 `Today, Mon Nov 16` / `Fri Nov 13`，多天是 `Last 30 days · click a bar to see its hours`。
- 颜色按上文加在 `style.rs`，另加 `token_web_search()`、`share_bar()`（项目占比条）和 `warn()`（未计价的提示）；涨跌直接用 `cost_up()` / `cost_down()`，没有另加 `delta_up` / `delta_down`。
- 测试名与上文一致；另加 `usage_view.rs` 的 `the_cost_breakdown_and_the_model_table_add_up_to_the_cost`（6）、`sessions_are_listed_by_their_cost_inside_the_range`、`a_range_s_bounds_follow_the_four_o_clock_days`（4），`keyboard_tests.rs` 的 `the_view_its_range_and_the_chart_metric_come_back_after_a_restart`（2）、`todays_cost_in_the_status_bar_opens_todays_usage_and_matches_it`（1、3）、`open_usage_in_the_menu_bar_menu_shows_the_window_on_usage`（1）、`a_hidden_usage_view_scans_every_ten_minutes`。点击类入口（工具栏切换、状态栏 Today、项目行）在测试里直接调点击处理函数调用的方法，没有模拟点击：gpui-kit 的 `click(id)` 只认登记了 `.test_support()` 的元素。

2026-11-16 审阅返工：修复较短上一月的终点裁剪及 DST 下同钟点比较。聚合在范围判定后才计价；空序列直接返回，有序非重叠槽位按二分定位（O(H log S)），任意重叠/乱序槽位保留原语义。沿用同一真实账本副本、release 构建和 1,000 次测量，30 天 summary + series 平均 **1.326 ms**（之前 2.512 ms）；证据 `tmp/2026-11-12-usage-stats/usage-p3-core/r1-bench.log`。公开 API 不变。

### 图表与 autoplay 实现与差异（usage-p3-chart，2026-11-16）

实施步骤 7、9 的脚本部分、10 的 `gpui-kit-guide.md` 和 `product-prd.md` 已做；实机验收、`AGENTS.md` 留给协调者。图表不加焦点和保存项，`app-architecture.md` 的两张表不用改。交接见 [P3 chart 总结](../../sessions/2026-11-16-usage-p3-chart.md)。

- **绘制**：`usage_chart.rs` 的 `UsageChart` 自己实现 `Plot`（`#[derive(IntoPlot)]` 可用，不用手写 `Element`），零件是 `Grid`、`PlotAxis`、`shape::Bar`、`shape::Line`、`PlotLabel` 和 `paint_quad` / `paint_path`。几何全在纯函数 `Layout` 里：绘图区左 56、上 22、下 28，右边按小时 20、按天 86（放均线的值）；柱宽是槽宽的 64%，在 3 到 22（小时）/ 26（天）之间，且不宽过槽（返工 R4，见下）。
- **堆叠**：只有最上一段有 3px 圆角（顶段和其余段分两次 `Bar` 画），段与段之间留 1.5px 白缝，值为 0 的模型不画。风险里担心的「整根同一圆角」这样解决。
- **前一天的虚线框**：`outline(.., BorderStyle::Dashed)` 画一个圆角 3 的 quad，比柱子宽 3px，在柱子后面；前一天那一小时为 0 时不画。
- **7 日均线**颜色是 `chart_average()`（正文色 75% 不透明，界面稿的 `#1d1d1f` .75），不是上文颜色表写的 `chart_ghost()`：灰色虚线压在蓝柱上看不清。`chart_ghost()` 只给虚线框。另加 `chart_axis()`（`#d8d8dd`，横轴）和 `chart_weekend()`（`#b0b0b6`，周末的日期标签）。
- **Hit rate 折线**用 Opus 的蓝（`model_color(0)`）加白心圆点，和视图那一期已有的图例 `hit rate` 一致；界面稿是深灰 `#3a3a40`。纵轴下限由快照按数据算：真实账本 11-01 有 2 次调用、缓存读取都是 0，30 天的 Hit rate 图因此从 0% 开始，是数据如此。
- **横轴标签**：小时每 3 个标一个（04、07、10…）；天从最后一天往前数，槽宽不够 44pt 时隔几天标一个，所以今天总有标签。今天的标签用正文色（界面稿是粗体，`PlotLabel` 忽略 `font_weight`），周末用 `chart_weekend()`。
- **今天的柱子**上标 `so far`（界面稿是 `截至 HH:MM`，按上文 PRD 用 `so far`）；现在线是 `accent()` 的 2/2 点线，上方标时刻（`17:16`）。
- **悬停明细**：标题 `Mon Nov 16 · 09:00–10:00`（小时）、`Fri Nov 13`（天）、`Mon Nov 16 · so far`（今天）；各模型一行（只列有值的，带色块），分隔线，`Total`、`In · hit · Out`、`Calls`，按小时加 `Day before`，按天加 `7-day average`；底部小字：未来的小时 `Not yet`，没有调用 `No calls`，按天且有调用 `Click to see its hours`。界面稿是中文，应用界面是英文。所在列用 `CrossLine::band` 加浅底。
- **`UsageState::hover` 的含义改成「钉住」**：鼠标悬停的状态在 Plot 元素内部，外面读不到也设不了；`hover` 是 autoplay 和测试钉住的槽位，鼠标不在图上时由 `Plot::prepaint` 画出它的明细。它不存进 `state.json`。
- **点击**：Plot 没有点击回调，外层 div 的 `on_mouse_down` 交给 `AppRoot::click_chart`，用 `paint` 记下的 bounds（`UsageState::chart_bounds`）和同一个 `Layout::slot_at` 换算；只有按天的图显示手形指针。
- **图例只列出图上有的虚线**：按小时时前一天没有调用就不列 `day before`，按天时均线全为 0 就不列 `7-day average`（`UsageSnapshot::build`）。范围内没有调用时只剩空坐标轴，不画 0 处的均线和 `7-day avg $0.00`。
- **图的标题行可以换行**：900pt 宽时图例原先被挤出窗口右边，现在换到第二行。
- **autoplay 用新变量 `ASM_USAGE_VIEW_AUTOPLAY=<gate 目录>`**，不扩展 `ASM_USAGE_AUTOPLAY`：后者的 gate 驱动改数据的扫描和重建（`make screenshot-usage` 靠它），Usage 视图的导览是固定顺序、只切视图状态。步骤：`today`、`today-hover`、`yesterday`、`d30`、`d30-in`、`d30-hit`、`d30-hover`、`drill`、`back`、`project`、`month`、`narrow`（`window.resize` 到 900 宽，截完恢复）、`sessions`、`empty`（2000-01-03 到 01-09 的 Custom，同时设进日期选择器）；`ASM_USAGE_VIEW_STEPS=a,b` 只跑其中几步。每步打 `autoplay-usage-view: <step> · view=… range=… metric=… chart=… hover=… empty=… calls=… cost=… project=… width=… <状态栏> today-matches=…`，等 gate 里的 `<step>.shot` 再走。脚本是 `scripts/screenshot-usage-view.sh`（`make screenshot-usage-view`），另用空数据目录启动一次截 `No usage recorded yet`；它只要求 `ASM_DB` 是副本（`shot-lib.sh::require_db_copy`）。上文「实机验收」里的 `ASM_USAGE_AUTOPLAY=1` 和 `screenshot-usage.sh` 以此为准。
- **测试**：`usage_chart.rs` 7 个（上文两个之外，`values_map_onto_the_plot_area_from_the_bottom`、`hours_are_labeled_every_third_and_days_count_back_from_today`、`the_tooltip_of_an_hour_lists_its_models_its_tokens_and_the_day_before`、`the_tooltip_of_a_day_has_the_average_and_says_a_click_opens_it`、`the_legend_lists_the_dashed_marks_only_when_there_are_some`）；`keyboard_tests.rs` 2 个（`clicking_a_day_s_bar_opens_its_hours_and_other_clicks_do_nothing`、`a_bar_shows_its_tooltip_under_the_mouse_or_when_pinned`），用 `VisualTestContext::simulate_click` / `simulate_mouse_move` 按坐标走 GPUI 的鼠标分发，不是直接调方法。
- **常驻菜单的点击偏移**：加了 Open Usage 后在菜单截图上重量，Open Sessions +101、Open Usage +125、Scan Now +148；`screenshot-resident.sh` 原先推算的 148 是对的，`RESIDENT_CLICK=1` 两次真点击都选中了对的项。

### 返工 R1（usage-p3-chart，2026-11-16）

处理审阅（`usage-p3-rv-app`）的 R1–R4 和实机验收（`usage-p3-accept`）发现的第 1、2、4 条，交接见 [P3 chart 总结](../../sessions/2026-11-16-usage-p3-chart.md) 末节。每条先写了在旧代码上失败的测试。

- **对话框关闭后焦点给当前视图**（R1）：更换收藏目录、取消最后一份收藏的确认、导出 TXT 的对话框关闭后调 `focus_view`，不再固定给表格。Usage 里从 Favorites 菜单换目录后，焦点原先落在不在屏幕上的表格，`⌃1` 之类的键到不了应用。测试 `changing_the_favorites_folder_from_usage_gives_the_keys_back_to_usage`。
- **Model 过滤两边用同一个名字**（R2）：`set_model_filter` 存去掉日期和 `[1m]` 的名字（`pricing::normalize`），`SessionRow::primary_model` 也存成这样，会话表的 Model 下拉随之把 `claude-haiku-4-5-20251001` 和 `claude-haiku-4-5` 并成一项；旧版本存在 `state.json` 里的带日期的值，`Filter::matches` 比较前同样去掉。原先在 Usage 里选 Haiku 4.5 切回会话表，主模型就是它的 session 也不见了。「Usage 按调用、会话表按主模型」这个含义上的差别没改（上文「风险与待定」）。测试 `a_model_picked_in_usage_keeps_its_sessions_in_sessions`、`the_model_filter_matches_a_main_model_with_or_without_its_date_or_window`。
- **虚线框按钟点对**（R3）：夏令时那天有 23 或 25 个小时，第 n 个小时不再是前一天的第 n 个。`same_clock_hours` 按本地钟点配对：某个钟点第 k 次出现，对前一天这个钟点的第 k 次；前一天没有的钟点（拨快那晚的 02:00）不画框，拨慢那晚第二个 01:00 也不画框（前一天只有一个 01:00）。测试 `the_box_behind_an_hour_is_the_same_clock_hour_the_day_before_across_dst`（America/New_York 2026-03-07、03-08、10-31、11-01，`chrono-tz` 加进 app 的 dev-dependencies）。
- **柱子不宽过自己的槽**（R4）：最小柱宽改成 `min(3, 槽宽)`。长的 Custom 范围（800px 画 365 天，槽宽 1.8px）原先柱宽 3px，伸进隔壁那天，鼠标在看得见的柱子边上时提示框和点击对的是另一天。测试 `a_bar_stays_inside_its_band_however_many_days_there_are`（365、730、1,000 天）。
- **工具栏换行**（验收第 1 条）：Usage 工具栏 `flex_wrap`，高度至少 36、随行数增加；数据时刻和刷新按钮包在一起、`ml_auto`，在哪一行都靠右。无界面测试窗口里量过：一行放得下要 1,140pt（选了项目略宽，打开日期选择器要 1,330pt），再窄就换行；一直窄到 300pt，Model 和刷新按钮都还在窗口里。范围按钮那一组不会再拆开，窗口比它还窄时它自己会伸出去。会话表的工具栏没动。测试 `the_usage_toolbar_wraps_in_a_narrow_window_instead_of_running_off_it`（1400 / 900 / 700pt，带项目、带日期选择器）。
- **纵轴刻度的小数位数跟着步长**（验收第 2 条）：`fmt_axis` 按步长决定小数：费用不到 1 美元的步长写够位数（`$0.0005`、`$0.005`，有小数时至少两位），0 写 `$0`；千以上 `$1.5k`；token 和命中率去掉多余的 0（`1.5k`、`99.25%`）。token 的步长只取 1、2、5 × 10ⁿ 且至少为 1（2.5k 用 `fmt_tokens` 写成 `3k`，500 的步长会把 1,500 和 2,000 都写成 `2k`）。原先一个小时的 Haiku 5.5 刻度是四个 `$0.00`。测试 `the_value_axis_says_each_line_s_value_and_no_two_alike`（一次到几千万次调用的 Cost / In / Out、空范围、11 种命中率，每个刻度读回去等于它的值、互不相同）。
- **提示框不盖图例**（验收第 4 条）：`Tooltip` 把框放在光标旁边，光标在图的下半部时框往上开，比光标以上的空间高就伸出图外，盖住标题行和图例。现在只取鼠标的 x，框的顶边固定在绘图区顶边，比图高时往下伸进下面的区块。钉住的提示框同样。测试 `a_bar_s_tooltip_never_covers_the_chart_s_legend`。
- **不在今年的范围写出年份**（P3 chart 总结交给协调者的那条）：Custom 在别的年份写 `Jan 3 – Jan 9, 2000`，跨年两端都写 `Dec 28, 2025 – Jan 3, 2026`，单日和点进来的某一天写 `Wed Dec 31, 2025`；图的副标题、工具栏里点进来那天的小标签同样。今年的不变。测试 `a_range_outside_this_year_says_its_year`。

