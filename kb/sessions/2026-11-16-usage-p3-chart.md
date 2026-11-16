---
created: 2026-11-16
tags: [usage, chart, gpui, screenshots]
---

# Usage 第 3 期：主图、Usage 视图截图脚本、常驻菜单点击偏移、文档

## 协调者摘要（usage-p3-chart）

- 结果：spec 步骤 7（`usage_chart.rs`：按模型堆叠的柱、前一天虚线框、7 日均线、Hit rate 折线、现在线、悬停明细、点一天看小时）、步骤 9 的脚本部分、步骤 10 的指南和 PRD 已完成；完成标准全部达到。
- 提交：本文件与代码、测试、脚本、文档、spec 同一提交，hash 见最终回复。
- 测试：`cargo test -p agent-session-manager` 211 passed / 0 failed / 2 ignored（新增图表 7 个、鼠标测试 2 个）；`make test` 合计 591 passed / 0 failed / 9 ignored。几何测试先红后绿（`red-geometry.log`），6 个变异里 5 个被抓到，剩下 1 个是等价变异（`mutations.log`）。
- 截图：`make screenshot-usage-view` 在真实账本副本上 15 张、全部判定通过、退出码 0（`screenshots/run2/`，`run2.log`）；Today $613.18 等于状态栏，30 天 $9,072.71 / 104,494 次调用，视图重算 0.9–3.4 ms。
- `make screenshot-usage`、`make test-screenshot-resident` 仍通过；`RESIDENT_CLICK=1` 真点击 Scan Now、Open Sessions 都选中了对的项，偏移重量为 101 / 125 / 148。
- 证据目录：`tmp/2026-11-12-usage-stats/usage-p3-chart/`（各 `.log`、`screenshots/{run1,run2,resident-measure,resident-click,usage-p1}/`）。
- 自己的决定：用新变量 `ASM_USAGE_VIEW_AUTOPLAY`，不扩展 `ASM_USAGE_AUTOPLAY`；`UsageState::hover` 改为「钉住」的槽位；均线用 `chart_average()`、Hit rate 用蓝色；图例只列图上有的虚线；标题行可换行。
- 未验证：真实鼠标的悬停和点击（只在无界面窗口里模拟过）、账本不可用和未计价两种空状态的界面、验收 11（60 秒内出现新调用）。
- 交给协调者：`AGENTS.md` 补新变量和新命令；实施步骤 7、9、10 的勾选；见下文「交给协调者」。

## 做了什么

**图表（`crates/app/src/usage_chart.rs`，新文件）。** 自己实现 gpui-component 的 `Plot`，不用现成的 `BarChart`。几何都在纯函数 `Layout` 里（绘图区、柱子位置、`slot_at`、`y`、哪些槽位标标签），画图、悬停明细、点击都只经它换算。按小时：24 根柱、每 3 小时一个标签、柱子后面是前一天同一小时的虚线框、今天有现在线。按天：每天一根、从今天往前隔几天标一个标签、周末标签变浅、今天标 `so far`、7 日均线（右边标最后的值）。Hit rate 画蓝色折线。悬停明细列出各模型、合计、In · hit · Out、调用数、前一天或 7 日均值，按天时提示可点击。`AppRoot::click_chart` 处理点击。

**视图（`usage_view.rs`）。** 图表区放进 `UsageChart`，外层 div 接收点击（只有按天时显示手形指针）；`UsageState` 加了 `chart_bounds`；图例只在图上有虚线框 / 均线时列出；标题行可以换行。加了 autoplay（`usage_view_autoplay`，14 步，每步打一行状态并等脚本截完图）。测试模块改成 `pub(crate)`，好让 `usage_chart` 的测试共用造数据的函数。

**其他代码。** `main.rs` 接 `ASM_USAGE_VIEW_AUTOPLAY`；`style.rs` 加 `chart_axis()`、`chart_weekend()`、`chart_average()`；`keyboard_tests.rs` 加两个鼠标测试。

**脚本。** 新 `scripts/screenshot-usage-view.sh` 和 `make screenshot-usage-view`；`shot-lib.sh` 加 `require_db_copy`（只要求索引是副本，记录目录可以是真实的）；`screenshot-resident.sh` 只改了偏移的注释（148 量过是对的）。

**截图后修的三处。** 看截图发现：没有调用的范围（14-empty）画了一条 0 处的均线和 `7-day avg $0.00`，图例也列着 `7-day average`；同一张图里日期选择器还显示着默认区间，没跟着 autoplay 设的 2000 年区间变；900 宽时图例被挤出窗口右边（12-narrow）。改成：没有可画的虚线就不画也不列图例，autoplay 的 `empty` 步骤同时设进日期选择器，标题行 `flex_wrap`。run2 复查都对了。图例改动补了测试，变异检查确认它能抓到旧逻辑。

## 测试对应

| spec 用例 / 验收项 | 测试 |
|---|---|
| `slot_at_maps_the_cursor_to_the_bar_under_it`（8） | `usage_chart::tests`，含左侧标签区、柱间空白、最右缘 |
| `stacked_segments_start_where_the_one_below_ends` | 同上 |
| 另加 | `values_map_onto_the_plot_area_from_the_bottom`、`hours_are_labeled_every_third_and_days_count_back_from_today`、`the_tooltip_of_an_hour_lists_its_models_its_tokens_and_the_day_before`、`the_tooltip_of_a_day_has_the_average_and_says_a_click_opens_it`、`the_legend_lists_the_dashed_marks_only_when_there_are_some` |
| 点击（8） | `keyboard_tests::clicking_a_day_s_bar_opens_its_hours_and_other_clicks_do_nothing`：`simulate_click` 点某天的柱子进入那天按小时；点标签区、按小时的图、没有调用的日子都不变 |
| 悬停（8） | `keyboard_tests::a_bar_shows_its_tooltip_under_the_mouse_or_when_pinned`：`simulate_mouse_move` 到柱子上出现明细框（`debug_selector`），移开消失，设 `hover` 后不靠鼠标也出现 |

变异（`mutations.log`）：去掉点击处理、去掉钉住、`tooltip_state` 返回 None、`slot_at` 不减左边距、旧的图例逻辑，都有测试失败；`slot_at` 的 floor 加一是等价变异（之后按柱子边缘校正，结果不变）。

## 截图与界面稿的差异

截图在 `screenshots/run2/`，界面稿是 `tmp/2026-11-12-usage-stats/mockup/usage-design.src.html`。几何（左 56、右 20 / 86、上 22、下 28、柱宽 64% 限 3–22 / 26、标签间隔、虚线框比柱宽 3px）照界面稿。不同之处：

- 悬停明细是英文（`Total`、`Calls`、`Day before`、`7-day average`、`Click to see its hours`、`Not yet`、`No calls`），界面稿是中文；今天的标记是 `so far`，界面稿是 `截至 HH:MM`（spec PRD 写的是 `so far`）。
- 悬停所在列的浅底是灰色（`CrossLine::band`），界面稿是 6% 的蓝。
- 今天的日期标签只换成正文色，没有加粗（`PlotLabel` 忽略字重）。
- Hit rate 折线是蓝色带白心圆点，界面稿是深灰 `#3a3a40`。
- 虚线框用 GPUI 的虚线边框画，虚线的间距由 GPUI 定，比界面稿的 `3 2` 略疏。
- 30 天的 Hit rate 纵轴从 0% 开始：真实数据 11-01 有 2 次调用、缓存读取都是 0。spec 说「不从 0 开始」指的是按数据定下限，这里数据就到 0。
- 900 宽时图例换到第二行（界面稿没有画窄窗口）。

## 给验收的做法

准备数据（不碰真实数据目录）：

```bash
D=$PWD/tmp/2026-11-12-usage-stats/<验收目录>
mkdir -p $D/data
# 先确认没有 app 在跑（pgrep -fl agent-session-manager），账本只读备份
sqlite3 "file:$HOME/Library/Application Support/dev.reorx.AgentSessionManager/usage-ledger.sqlite?immutable=1" ".backup $D/data/usage-ledger.sqlite"
# 今天的记录文件 APFS 克隆（~/.claude/projects 是符号链接，find 要带尾斜杠）
```

导览：`ASM_DB=$D/data/index.sqlite ASM_CLAUDE_DIR=<副本或真实 ~/.claude> ASM_FAVORITES_DIR=$D/data/favorites ASM_TRASH_DIR=$D/data/trash make screenshot-usage-view SHOT_DIR=$D/screenshots`。每步的日志行 `autoplay-usage-view: <step> · …` 在输出和 `app-*.log` 里，`usage view: <范围> · N calls · $X · built in Nms` 是每次重算。`ASM_CLAUDE_DIR` 用真实目录时 app 只读它，但会把新用量写进副本账本，这正是验收 11 需要的。

| 验收项 | 怎么验 |
|---|---|
| 1 进入 Usage | 键盘、View 菜单、状态栏 Today、Open Usage 都有键盘测试。真实操作：导览的 `sessions` 步骤证明切回；工具栏切换、状态栏 Today 要真点击（cliclick，点前按 brief 确认点击处的窗口属于本应用）；菜单栏的 Open Usage 可用 `screenshot-resident` 弹出的菜单，偏移 +125（脚本只点 Scan Now 和 Open Sessions，Open Usage 要自己点） |
| 2 重启后恢复 | 导览结束时进程是被 kill 的，不能当证据。手动：固定 `ASM_STATE`，切到 Usage / 30 days / In，`⌘Q`，再启动看 |
| 3 Today = 状态栏；项目 | `today` 行 `today-matches=yes`；`project` 行的 `cost=` 等于 `04-d30.png` 按项目表里该项目的费用（run2：vibench $4,206.18 两边一致） |
| 4 4:00 分日 | `d30` 行 `chart=daily/30`；用 `sqlite3` 在副本账本上按 `hour` 加 4 小时偏移汇总某一天，和 `08-drill.png` 的卡片对 |
| 5 上一段 | 卡片的小字在每张图上；`no earlier data` 见 `14-empty.png` |
| 6 加总 | 单元测试；截图里钱花在哪、按模型的合计和 Cost 一致 |
| 7 24 根柱 / 每天一根 / 均线 / 四个指标 | `today` `chart=hourly/24`（夏令时日 23 / 25）、`01-today.png` 从 04 开始；`d30` 有均线；`d30-in`、`d30-hit` 行 `metric=in` / `metric=hit`，Out 没有单独一步 |
| 8 悬停与点击 | `today-hover`、`d30-hover` 行 `hover=<槽位>`，`02`、`07` 截图有明细框（钉住，不是鼠标）；`drill` / `back` 走的是点击与 Escape 后的同一方法。**真实鼠标**的悬停、点柱子、点 ✕ 只在无界面测试里模拟过，要用 cliclick 实点（图在窗口里的位置随窗口宽度变，先截图量） |
| 9 过滤保持 | `month`、`sessions` 行 `project=` 没被清掉，`13-sessions.png` 的 Project 下拉是该项目；Model 过滤要点下拉 |
| 10 记录已清理的 session | `ASM_CLAUDE_DIR` 用只含今天记录文件的副本时，30 天费用最高的 session 大多没有记录文件，`04-d30.png` 里它们的标题和项目照常显示 |
| 11 60 秒内出现 | 需要 `ASM_CLAUDE_DIR` 指向真实目录。Usage 开着，另开终端跑一次 Claude Code，看日志的 `auto-scan:` 每分钟一次、随后的 `usage view: …` 调用数变大 |
| 12 四种空状态 | 无调用：`14-empty.png`（`empty=no-calls`）；空账本：`15-no-usage.png`（`empty=no-usage`）；账本不可用：复制一份账本、`PRAGMA user_version = 2`，`empty=unavailable`；未计价：单元测试，界面上需要账本里有价格表没有的模型 |
| 13 会话表专用键无效 | 键盘测试，不需要实机 |

## 交给协调者

- `AGENTS.md`：开发命令的截图一条加 `make screenshot-usage-view`（要设 `ASM_DB`，账本放在它旁边）；环境变量一条加 `ASM_USAGE_VIEW_AUTOPLAY`（gate 目录，Usage 视图的导览）。
- spec 实施步骤 7、9（脚本部分）、10（指南、PRD）可以勾；步骤 9 的实机验收和步骤 10 的 `AGENTS.md` 没做。`app-architecture.md` 不需要改：图表不加焦点，也不加保存项。
- 一个小问题，不是本期的代码：不在今年的范围，标题只写 `Jan 3 – Jan 9`，不带年份（日期选择器里有完整日期）。要不要改由视图那一期的 owner 定。
- 真实账本只到 11-14 17:00 左右（推测：导入之后真实数据目录里还没跑过 P1 之后的构建）。自查里 11-14 晚上以后的数字都来自 `claude-today/`（今天改过的 146 个记录文件）扫进副本账本的部分，所以 11-14、11-15 可能比实际偏少；验收用真实 `~/.claude` 扫一遍就完整了。

## 踩过的坑

- **`slot_at` 的 floor 加一是等价变异**：后面会按柱子边缘把结果校正回来，测试抓不到也不该抓到。要验证 `slot_at`，变异改左边距或带宽。
- **截图才发现的问题**：空范围的 0 均线、日期选择器没跟着变、窄窗口图例被裁，单元测试和键盘测试都看不出来。截图逐张用 `Read` 看过才找到。
- **`PlotAxis` 的 `x_label` / `y_label` 在调用时就算位置**：先设 `x` / `y` / `y_label_side`，再给标签，否则标签不显示或在错的一边。
- **悬停状态在 Plot 元素内部**：外面读不到也设不了，autoplay 截明细只能钉住（`Plot::prepaint` 自己画一个）。
- **`~/.claude/projects` 是符号链接**：`find ~/.claude/projects -newer …` 什么都找不到，要写 `projects/`。
- 账本表名是 `usage_hourly`，`hour` 是 UTC 整点（`ts / 3_600_000`）。

## 相关文档

- [Usage 视图 spec](../plans/2026-11-16-usage-stats/usage-view.md) — 本期依据，新增「图表与 autoplay 实现与差异」一节
- [用量统计总 plan](../plans/2026-11-16-usage-stats/index.md) — 口径（4:00 分日、按调用时间归日）
- [P3 view 总结](2026-11-16-usage-p3-view.md) — 图表区的交接（`ChartData`、`drill_into_day`、`hover`）
- [P3 core 总结](2026-11-16-usage-p3-core.md) — 聚合接口
- [gpui-kit 开发指南](../docs/gpui-kit-guide.md) — 更新：新增「自绘图表：`Plot`」一节、`screenshot-usage-view`、常驻菜单的偏移
- [产品 PRD](../docs/product-prd.md) — 更新：Usage 视图状态和图表描述
- [应用架构](../docs/app-architecture.md) — 返工 R1 更新：对话框关闭后的焦点、过滤共用的 Model 名

## 协调者摘要（usage-p3-app-r1）

- 结果：任务 `usage-p3-app-r1` 的 5 项都做完：审阅 R1–R4、实机验收第 1、2、4 条、范围标题的年份、`winid --at` 的层级、指南的 agent-desktop 一节。完成标准全部达到。
- 提交：代码、测试、脚本、文档、spec、本节同一提交，hash 见最终回复。
- 先红后绿：新加 10 个测试，在旧代码上全部失败（`r1-before.log`，每条的失败信息都对得上问题），改后全过（`r1-after.log`）。
- 测试：`cargo test -p agent-session-manager` 220 passed / 0 failed / 2 ignored（`r1-app-tests.log`）；`make test` 合计 600 passed / 0 failed / 9 ignored（`r1-make-test.log`）；`make test-screenshot-resident` 通过。
- 截图：`make screenshot-usage-view` 15 张，全部判定通过，退出码 0（`screenshots/r1/`，`r1-tour.log`）。打开看过：`12-narrow` 工具栏换行，Model、`as of`、刷新都在窗口里；`02`、`07` 提示框从绘图区顶端往下开，不盖图例；空范围标题是 `Jan 3 – Jan 9, 2000`。
- 小数值的刻度：Haiku 当天 $0.02 的图，刻度是 `$0 / $0.005 / $0.010 / $0.015 / $0.020`（`r1-haiku/01-today.png`）。这次只跑了 `today` 一步，脚本的 `today-matches` 判定按设计不过：过滤后的 Cost 不等于状态栏。
- `winid --at`：旧版对任何点都答 Dock，新版答出真实窗口，Dock 条上仍答 Dock（`winid-at.log`）。
- 自己的决定：
  - `winid` 只跳过 Dock 那个层级 20 的全屏窗口，而且只在屏幕可见区域内跳过；`topwin.py` 跳过 1–23 全部层级，会放过别的应用的浮动面板。
  - 工具栏用 `flex_wrap` 加 `ml_auto` 换行，不收起控件。
  - 夏令时那天，前一天没有的钟点和第二个 01:00 不画虚线框。
  - token 刻度的步长不取 2.5。
- 未验证：真实鼠标在右侧柱子上悬停（只在无界面窗口和钉住的截图里看过）；会话表工具栏在窄窗口下的样子。
- 交给协调者：见本节「交给协调者」。

### 做了什么（每条的测试）

| 条目 | 改法 | 测试（旧代码上的失败） |
|---|---|---|
| R1 焦点 | 更换收藏目录、确认取消最后一份收藏、导出 TXT 三个对话框关闭后调 `focus_view`，不再固定调 `focus_table` | `changing_the_favorites_folder_from_usage_gives_the_keys_back_to_usage`（焦点落到表格：`(false, true)`） |
| R2 Model 名 | `set_model_filter` 存 `pricing::normalize` 之后的名字；`SessionRow::primary_model` 也存成这样（Sessions 的下拉随之合并）；`Filter::matches` 比较前把旧 state 里带日期的值也规范化 | `a_model_picked_in_usage_keeps_its_sessions_in_sessions`（会话表空）、`model::tests::the_model_filter_matches_a_main_model_with_or_without_its_date_or_window` |
| R3 夏令时 | `same_clock_hours`：按本地钟点配对，钟点第 k 次出现对前一天的第 k 次；前一天没有的钟点不画框 | `the_box_behind_an_hour_is_the_same_clock_hour_the_day_before_across_dst`（New York 03-07 的 03:00 对到 02:00）；app 加 `chrono-tz` dev 依赖 |
| R4 柱宽 | 最小柱宽 `min(3, 槽宽)` | `a_bar_stays_inside_its_band_however_many_days_there_are`（365 天柱子出槽） |
| 验收 1 窄窗口 | Usage 工具栏 `h_auto` + `min_h(36)` + `flex_wrap`，时刻和刷新包成一组 `ml_auto`。测试窗口里量：1,140pt 以下换行（日期选择器打开时 1,330pt），到 300pt 时 Model 和刷新都还在窗口里 | `the_usage_toolbar_wraps_in_a_narrow_window_instead_of_running_off_it`（900pt 时 Model 在 x=932） |
| 验收 2 刻度 | `fmt_axis` 按步长决定小数位数（`decimals`、`axis_number`、`unit_of`）；token 步长只取 1/2/5 × 10ⁿ 且至少为 1 | `the_value_axis_says_each_line_s_value_and_no_two_alike`（四个 `$0.00`） |
| 验收 4 提示框 | 只把鼠标的 x 交给 `Tooltip`，y 固定在绘图区顶端减 gap，所以框顶边就是绘图区顶边；钉住的提示框同样 | `a_bar_s_tooltip_never_covers_the_chart_s_legend`（框顶在绘图区上方 60px） |
| 年份 | `range_label` 带上今年的年份做比较：别的年份写 `Jan 3 – Jan 9, 2000`，跨年写 `Dec 28, 2025 – Jan 3, 2026`，单日写 `Wed Dec 31, 2025`；副标题和点进那天的小标签同样 | `a_range_outside_this_year_says_its_year`；`picking_a_custom_range_in_usage_…` 的 `Sep 1 – Sep 30` 改成按当前年份算期望值，明年也能过 |
| `winid --at` | 在 `NSScreen.visibleFrame` 内跳过 Dock 的层级 20 窗口，其余层级照算 | 手动探测（`winid-at.log`），`make test-screenshot-resident` 用替身不受影响 |
| 文档 | 指南：Plot 表加柱宽、提示框位置、纵轴刻度、`simulate_resize` 测布局；点击前确认的 Dock 说明；新增「agent-desktop 能做什么」。`app-architecture.md`：焦点表、过滤共用的 Model 名。PRD 4.11、spec 新增「返工 R1」一节 | — |

### 交给协调者

- `AGENTS.md`：GPUI 规则「新增弹出层，关闭后要把焦点还回去」可改成「还给当前视图（`focus_view`）」。在 Usage 里也能打开的对话框，把焦点还给表格就等于没有焦点（R1）。
- 会话表的工具栏在窄窗口下没量过。推测 900pt 时同样放不下（搜索框 220pt 加 Preview），不在本任务范围。
- core 的 `cost::primary_model` 按原始模型名分组：同一个 session 里 `claude-haiku-4-5-20251001` 和 `claude-haiku-4-5` 都出现时，费用会被分成两组，主模型可能判错。很少见，app 这边没法修。
- 以后的验收点击前用 `winid --at`，不要再用 `usage-p3-accept/topwin.py`：它跳过 1–23 全部层级，会放过别的应用的浮动面板。agent-desktop 对 .app 包的应用名没试过（debug 二进制是 `agent-session-manager`）。
- 上一节「交给协调者」里的年份一条已在本次修掉。
- 重新验收时要看：
  - 窄窗口的工具栏；
  - 真实鼠标悬停靠右、靠下的柱子；
  - 小数值的刻度；
  - 别的年份的范围标题。

### 踩过的坑

- **提示框测试第一版在旧代码上也通过。** 原因有两个：
  - fixture 的提示框只有 158px 高，比真实数据的矮；
  - 我取的「中线」是绘图区的中线（115），不是 `Tooltip` 判断开口方向用的图表中线（118）。

  鼠标放到图表中线以下才复现。改测试前先在旧代码上确认它会失败。
- **提示框内容的内边距是 6px，不是 8px**（主题的 rem 不是 16）。测试改成断言内容的顶边不高于绘图区顶边。
- **`fmt_tokens` 写 k 时取整**：1,500 写成 `2k`，所以刻度不能用 500 这种步长配 `fmt_tokens`，改用按步长取小数的 `axis_number`。
- **Usage 默认打开 Today**，而 fixture 的调用都在 9 月。断言 Usage 的总数前先设 Custom 范围。
- **索引复制到新的数据路径、又换了 `ASM_FAVORITES_DIR`**，启动会弹「数据目录已更改，正在重建索引」（`RebuildingRoots`），在副本上重建一次。这是副本的设置引起的，不是本次改动；账本照旧，数字和状态栏一致。
