# 用量统计第 3 期：Usage 视图（除图表）

## 协调者摘要（usage-p3-view）

- **结果**：spec 实施步骤 2–6、8 完成：`ViewState` 新字段；`UsageSnapshot::build`（纯函数）；`MainView` / `set_view` / `⌃1` `⌃2` / View 菜单 / 会话表专用动作在 Usage 下返回 / `⌘F` 回会话表；工具栏两种形态；`render_usage`（五张卡、图表占位、四块表、空状态、窄屏单列、纵向滚动）；状态栏 Usage 形态与可点的 `Today`、Usage 下每 60 秒扫描、`search_stale`、菜单栏 `Open Usage`。图表区是 `id("usage-chart")`、高 236 的空容器。
- **提交**：本总结所在的提交（`Add the Usage view without its chart`）。
- **测试**：`cargo test -p agent-session-manager` 202 passed / 0 failed / 2 ignored（新增 `usage_view.rs` 9 个、`keyboard_tests.rs` 14 个，`settings.rs` 两个用例加了断言）；`make test` exit 0，合计 579 passed / 0 failed / 9 ignored（`tmp/2026-11-12-usage-stats/usage-p3-view/make-test.log`）；`make test-screenshot-resident` 全过。三次变异（去掉 60 秒间隔、Usage 下照常重搜、去掉 Preview 的判断；去掉隐藏时换计时任务）都让对应测试变红。
- **截图**（都 Read 过）：`tmp/2026-11-12-usage-stats/usage-p3-view/screenshots/` 的 `1-usage-today`（范围内无调用）、`2-usage-30days`、`3-usage-custom-day`（Custom 形态的工具栏 + 单天 + Hit rate）、`4-sessions-bars`（会话表形态的工具栏与状态栏）、`5-usage-no-ledger`（空账本），`*-cards.png` / `*-tables.png` 是局部放大。数据：`index-copy.sqlite` 的 immutable 备份，app 打开时因 schema 4 重建、先 salvage 出账本（129,381 次调用，截至 11-14 17:00）。
- **自己的决定**：`search_stale` 用枚举（Refresh / Rerun）；加 `ResidentCommand::Reopen`（Finder 重开保持视图）；窗口隐藏 / 重显时换计时任务；窄屏阈值 960；Cost 用两位小数 `fmt_money`；无调用时「钱花在哪」不列四行 0；`screenshot-resident.sh` 的 Scan Now 点击偏移 124 → 148。全部写回了 spec 的「App 实现与差异」。
- **未验证**：窄窗口单列（不发按键改不了窗口大小）；真实点击（工具栏切换、范围按钮、项目行、状态栏 Today、Custom 选择器弹层、刷新按钮）；真机按 `⌃1` / `⌃2`；菜单栏 Open Usage 真点与 148 的偏移；验收 11（60 秒内新调用出现）。
- **交给协调者**：见下节 4 条，其中第 1 条（`ASM_USAGE_AUTOPLAY` 撞名）影响步骤 9。

## 测试名与 spec 步骤的对应

| 步骤 | 测试 |
|---|---|
| 2 settings | `settings.rs`：`a_saved_state_comes_back_unchanged`（新字段往返）、`fields_added_later_default_when_an_old_file_lacks_them`（Sessions / Today / None / Cost） |
| 3 build | `usage_view.rs`：`cards_show_cost_tokens_calls_and_sessions`、`the_cost_breakdown_and_the_model_table_add_up_to_the_cost`、`the_comparison_reads_no_earlier_data_without_a_previous_span`、`an_empty_ledger_and_an_empty_range_read_differently`、`unpriced_calls_are_called_out_on_the_cost_card`、`the_project_table_folds_the_tail_into_other_projects`、`sessions_are_listed_by_their_cost_inside_the_range`、`a_single_day_charts_hours_and_several_days_chart_days`、`a_range_s_bounds_follow_the_four_o_clock_days` |
| 4 切换与焦点 | `keyboard_tests.rs`：`control_2_shows_usage_and_control_1_goes_back_with_the_focus_following`（也测 View 菜单的 action）、`the_view_shortcuts_work_from_the_search_box`、`command_1_to_4_still_switch_the_preview_mode`、`command_f_in_usage_goes_to_sessions_and_focuses_the_search`、`session_only_shortcuts_do_nothing_in_usage`、`the_view_its_range_and_the_chart_metric_come_back_after_a_restart` |
| 5 工具栏 | `a_filter_set_in_usage_is_still_set_in_sessions`、`picking_a_custom_range_in_usage_hands_the_focus_back_to_usage`、`escape_leaves_a_drilled_day`；外观看截图 |
| 6 render | 截图 1–5 |
| 8 状态栏与节奏 | `todays_cost_in_the_status_bar_opens_todays_usage_and_matches_it`、`usage_scans_every_minute_and_sessions_every_ten`、`a_hidden_usage_view_scans_every_ten_minutes`、`a_reload_in_usage_defers_the_search_until_sessions_is_shown`、`open_usage_in_the_menu_bar_menu_shows_the_window_on_usage` |

点击类入口在测试里直接调点击处理函数调用的方法（`set_view`、`show_todays_usage`、`set_project`、`set_usage_range`、`drill_into_day`），没有模拟点击：gpui-kit 的 `click(id)` 只认登记了 `.test_support()` 的元素，应用里还没有元素这样登记。

## 与界面稿的差异（截图对照 `tmp/2026-11-12-usage-stats/mockup/usage-design.src.html`）

- 分段控件选中的那一格是白底加细边框，界面稿是白底加阴影。
- 点进某一天的小标签写 `Fri Nov 13 ✕`，界面稿是 `11-13 Fri ✕`；`as of 16:44` 不带日期，界面稿是 `数据截至 11-16 15:53`。
- 刷新按钮是 lucide 的 `rotate-cw` 图标，界面稿是 `↻` 字符。
- 下拉沿用会话表的 `Label value ▾` 外观，与界面稿一致。
- 图表区留白（步骤 7）。

## 交给协调者

1. **`ASM_USAGE_AUTOPLAY` 已被占用**：spec「测试与验证」写的是 `ASM_USAGE_AUTOPLAY=1` 逐步走 `today → yesterday → … → sessions`，但第 1 期已经把这个变量用作 gate 目录（`scripts/screenshot-usage.sh`，文件驱动扫描 / 重建）。做步骤 9 的人要换一个变量名或在 gate 机制上加步骤，`make screenshot-usage` 不能被改坏。
2. **会话表的 Model 过滤比的是 `row.primary_model == filter.model`（原样字符串）**，Usage 的 Model 下拉给的是 `pricing::normalize` 过的 id（core `model_entries`），`UsageFilter` 也按 normalize 比。只要 `cost::primary_model` 返回的 id 都是规范化的就一致；没有在真实数据上核对过。两边的含义本来就不同（会话表 = 主模型，Usage = 每次调用的模型），验收 9 接受这一点，但在 Usage 里选了一个小模型、回到会话表时表可能几乎空了，验收时值得看一眼。
3. **菜单栏菜单多了一项**：`resident.rs` 的序号变为 Open Sessions 4、Open Usage 5、Scan Now 6、Quit 8；`scripts/screenshot-resident.sh` 的 `RESIDENT_CLICK=1` 点 Scan Now 的偏移按一行 24pt 推成 148，没有实测。下一次跑 `RESIDENT_CLICK=1` 的验收要确认（点错时脚本会报 `the click did not choose ScanNow`，不会点到别的应用）。
4. **Finder 重开改发 `ResidentCommand::Reopen`**：日志里从 `menu bar: OpenSessions` 变成 `menu bar: Reopen`。`screenshot-resident.sh` 只在真点击时检查 `OpenSessions`，`RESIDENT_REOPEN` 路径不受影响；若有别的验收脚本 grep 这一行，要改。

## 给 usage-p3-chart

- **位置**：`usage_view.rs` 的 `AppRoot::render_chart_panel`。图表区是 `div().id(CHART_ID).w_full().h(px(CHART_H)).flex_none()`（`CHART_ID = "usage-chart"`，`CHART_H = 236.`）；账本为空（`Empty::NoUsage`）时里面写 `No usage recorded yet`，其余情况是空的，把 `UsageChart` 作为它的 child 放进去即可（`NoCalls` 时 `chart.slots` 照常有槽位，画空坐标轴）。标题、副标题、指标切换（`set_usage_metric`）和图例（`render_legend`，色块用 `style::model_color`）已经在面板头里。
- **数据**：`self.usage_view.snapshot.chart: ChartData`：`hourly`、`metric`、`slots: Vec<ChartSlot>`（`label`、`day`、`span`、`weekend`、`totals`、`by_model: [Totals; 5]`、`ghost: Option<Totals>`（单天：前一天同一小时）、`average: Option<f64>`（多天，Hit rate 时为 None）、`partial`（今天的柱子）、`future`（今天还没到的小时））、`y_min` / `y_max`、`ticks: Vec<(f64, SharedString)>`（最后一个恰好等于 `y_max`）、`legend`、`now: Option<(f64, SharedString)>`（单天且是今天：从左边数的槽位数，如 `11.88`，和 `15:53`）。`ChartSlot::value(metric)` 是柱高（Hit rate 是比例，没有 In 时 None），`ChartSlot::stack(metric)` 是五段堆叠（Hit rate 时全 0），`fmt_metric(metric, v)` 给提示里的数，这两个函数现在标了 `#[allow(dead_code)]`，用上后删掉标记。
- **悬停**：`UsageState::hover: Option<usize>` 已有，`drill_into_day` / `leave_day` 会清掉它。
- **点柱子**：调 `AppRoot::drill_into_day(slot.day, cx) -> bool`。它自己判断：按小时的图上、或那天没有调用时返回 false 什么都不改；成功时 `range = Day`、`back` = 之前的范围、重算快照。Escape 和小标签的 ✕ 已经接到 `leave_day`。
- 颜色：`style::model_color(slot)`、`chart_ghost()`（虚线框、均线）、当前时刻线用 `accent()`。

## 踩过的坑

- `UsageFilter` 没有 `PartialEq`（core 不归我改），`apply_filter` 判断 Usage 的过滤变没变用 `usage_view::same_filter` 逐字段比。
- 卡片小字用 `h_flex().flex_wrap()` 拼多个 div 时，单个长片段不会换行，会压到右边一张卡上（30 天的 In 卡）。改成一段 `StyledText` 加颜色区间后正常换行。
- `toolbar::toggle` 的文字是等宽字体、宽度固定 26，拿来做 `Custom…` 很难看；另写了 `tool_button`。
- `sips --cropOffset` 在这台机器上不生效（裁出来的是中间），局部放大改用 `magick -crop`。
- 自检数据：`index-copy.sqlite` 是 schema 4，用它的 immutable 备份当 `ASM_DB`，app 打开时先 `salvage_into_ledger` 再重建，正好得到一份真实规模的账本，不用碰真实数据目录。
