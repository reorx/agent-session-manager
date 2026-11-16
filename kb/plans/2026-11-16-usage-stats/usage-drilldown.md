---
created: 2026-11-16
tags:
  - usage
  - filter
  - navigation
  - agent-session-manager
  - plan
---

# 从 Usage 回到会话表：选中某个 session，或者看一段时间里有活动的 session

总 plan 是 [index.md](index.md)。这是第 4 期，依赖 [usage-view](usage-view.md)（Usage 视图、视图切换、共用的过滤）和 [usage-ledger](usage-ledger.md)（内存里的用量数据）。

## PRD

### 背景

Usage 视图告诉用户「这一天花了 $300，大头在某个项目的某几个 session 上」，下一步自然是去看那几个 session 做了什么。现在只能记下短 id，切回会话表自己找。

会话表的时间过滤也帮不上忙：它按 session 的最后更新时间筛。一个 10-25 开始、10-27 结束的 session，在 10-26 那天花了钱，但用「10-26」去筛是筛不出来的。

### 目标

在 Usage 里点一个 session，直接回到会话表并选中它；点一个项目或 session 数旁边的跳转，会话表列出这段时间里确实有调用的 session。

### 功能与交互

**点一个 session**

- 「费用最高的 session」里每一行都可以点（鼠标移上去整行高亮）。点了之后切到会话表，选中这个 session，表格滚到它，详情面板显示它，键盘焦点在表格上。
- 如果当前的过滤或搜索把它挡在外面，应用只放宽挡住它的那几项，其余不动，并弹一条通知说明放宽了什么，例如「已清除过滤：Updated、Model」。可能被放宽的有：搜索词（清空）、Updated（改为 Any time）、Project（改为 All）、Kind（把它的类型勾上）、Model（改为 All）、只看收藏（关掉）、不显示没有 prompt 的 session（改为显示）、「这段时间里有活动」（清掉）。
- 搜索只在结果确定还会显示它时保留：搜索已经结束、搜索框里的词就是搜过的词、它在结果里。新的词还在搜（或刚打完、还没开始搜）时，旧结果说明不了新词，搜索被清空，免得新结果到达时把刚选中的它换掉。
- 「这段时间里有活动」开着时，Model 按调用比（见下）：它在这段时间里调用过别的模型、没调用过选中的模型，放宽的是 Model；这段时间里一次调用都没有，放宽的是 Active（Model 回到按主模型比，它的主模型不是选中的那个时 Model 也放宽）。
- 这个 session 的记录文件已被清理、会话表里只剩 History 行时：照常选中，详情显示它在历史里的 prompt。
- 会话表里完全没有这个 session（记录已删，也不在历史里）：不切换视图，弹一条通知「这个 session 的记录已被清理，只保留了用量」。这种行的短 id 用灰色显示，鼠标移上去的提示写明原因。
- 这个 session 属于会话表里被隐藏的项目：不切换视图，弹通知「这个 session 的项目在会话表里被隐藏，可在 Project 菜单里恢复显示」。不替用户取消隐藏。

**看有活动的 session**

两个入口，都在 Usage 里：

- 「按项目」表每一行末尾的 `↗`：切到会话表，Project 设为这个项目，并加上「这段时间里有活动」的过滤，时间段是 Usage 当前的范围。
- 指标卡 Sessions 上的 `↗`：切到会话表，保留当前的 Project / Kind / Model，加上同样的过滤。范围是某一天时，这就是「看这一天有哪些 session」。

跳转之后：

- 会话表只列出在这段时间里至少有一次 API 调用的 session（算上它的 subagent 的调用）。
- Updated 被改为 Any time（否则两个时间条件会互相打架）；搜索词被清空。
- 工具栏在 Updated 后面出现一个小标签，写着这段时间，例如 `Active 11-12 ✕` 或 `Active 11-01 – 11-12 ✕`。点 ✕ 清掉这个过滤，其他过滤不变，焦点留在表格上。
- 这个过滤和其他过滤、搜索叠加：再选项目、再搜索，都在「这段时间里有活动」的范围内进行。
- 这个过滤开着时，Model 的含义换成「这段时间里调用过这个模型」，和 Usage 的 Model 一样按调用比，不再按 session 的主模型比（用户决定，见实现记录）。所以从 Usage 带着 Model 跳过来，列出的正是 Usage 算进去的那批 session，只缺记录已清理的和被其他过滤（收藏、空 session、隐藏的项目等）挡住的。跳转之后再换模型也一样按调用比；清掉这个过滤后，Model 回到按主模型比。
- 这段时间里有调用、但会话表里已经没有的 session（记录被清理）列不出来。有这种情况时，跳转后弹一条通知：「这段时间还有 N 个 session 的记录已被清理，未列出」。
- 这个过滤不随重启保留：它是一次跳转的上下文。
- 应用开着时扫到了新的调用，列表跟着更新（范围含今天时，新开始的 session 会出现）。
- 表格的排序不变。Cost 列仍是 session 的总费用，不是这段时间内的费用。

**按项目的行**

- 点行的其余部分仍然是在 Usage 里按这个项目过滤（usage-view 的行为）；只有点 `↗` 才跳转。

### 不做的事

- 不在会话表里加「这段时间内的费用」列，也不按它排序。
- 不做从主图的某根柱子直接跳到会话表（先点进那一天，再点 Sessions 卡上的 `↗`）。
- 不做从「按模型」表跳转。
- 不在会话表的工具栏里提供手动设置「这段时间里有活动」的入口；它只能从 Usage 带过来。
- 不为记录已清理的 session 提供只读的用量详情页。
- 不记住跳转前 Usage 的状态以外的东西：切回 Usage 时范围和过滤还是原来的（Project 可能已被跳转改了，因为它是共用的）。

### 验收标准

1. 点「费用最高的 session」里的一行：会话表选中它，表格滚到它，详情显示它，焦点在表格上（方向键、`⌘C` 立即可用）。
2. 它被过滤或搜索挡住时，只有挡住它的过滤被放宽，通知列出了被放宽的项。
3. 会话表里没有的 session：留在 Usage，出现通知；这种行显示为灰色。
4. 项目被隐藏的 session：留在 Usage，出现通知，隐藏设置不变。
5. 点项目行的 `↗`：会话表的 Project 是这个项目，Updated 是 Any time，出现 `Active …` 标签，列出的每个 session 在这段时间里都有调用，且这个项目在这段时间里有调用的 session（会话表里存在的）都在列。
6. 一个最后更新在范围之后、但范围内有调用的 session 出现在列表里；一个最后更新在范围内、但范围内没有调用的 session 不出现。
7. 点 Sessions 卡的 `↗`：Project / Kind / Model 不变，加上 Active 过滤。
8. 点标签的 ✕：过滤清掉，其他过滤不变，焦点在表格上。
9. Active 过滤开着时搜索，结果都在过滤后的范围内；清掉过滤后搜索重跑。
10. 有记录已清理的 session 时，通知里的 N 等于 Usage 的 Sessions 数减去列表里的行数（开着 Model 过滤时也成立）。
11. 重启后没有 Active 过滤，Updated 是跳转时设的 Any time。

## Tech Design

### 现状

- usage-view 之后：`AppRoot.view`（`MainView`）、`set_view`、`usage_view: UsageState`（范围、`UsageSnapshot`），`usage: UsageData`（core `usage.rs`，`BTreeMap<session id, SessionUsage>`，每个桶有 UTC 小时）。「费用最高的 session」和「按项目」的行来自 `Summary.by_session` / `by_project`，带 session id 和项目 key。
- `crates/app/src/model.rs::Filter`：`favorites_only`、`project`、`since_ms` / `until_ms`（比的是 `SessionRow.updated_at`）、`show_empty`、`kinds`、`model`、`hidden_projects`；`Filter::matches(&SessionRow)` 是纯函数。
- `crates/app/src/table.rs::SessionTableDelegate`：`rows` 是全部行，`ix` 是过滤排序后的顺序；`set_filter` 返回过滤是否变了并 `recompute`；`shows(row)` 是 `Filter::matches` 加搜索命中；`position_of(id)` 在 `ix` 里找；`search_targets()` 从 `ix` 生成搜索对象。
- `crates/app/src/toolbar.rs`：`apply_filter` 应用过滤、`after_rows_changed`、`persist`，搜索中则 `run_search`；`set_range` 把 `RangePreset` 换成边界写进过滤；`clear_search` 清空搜索框并把焦点给表格。
- `crates/app/src/main.rs`：`select_row(row_ix, cx)` 选中并在换了 session 时读对话；`after_rows_changed` 用 `wanted_id` 等待一个还没出现的行；通知用 `window.push_notification(..)`。
- 选中会滚动：`TableState::set_selected_row` 顺带滚到该行，正是这里要的。

### 方案

**跳到一个 session**：`AppRoot::reveal_session(id)`。先在表格的全部行里找这个 id，找不到或项目被隐藏就通知并返回。找到后用一个纯函数算出「为了让这一行显示，要放宽哪些过滤」，应用放宽后的过滤（必要时清空搜索），切到会话表，选中它。

**这段时间里有活动**：`Filter` 加一个字段记时间段；表格的 delegate 另外持有一个「这段时间里有调用的 session id 集合」，由 `AppRoot` 从 `UsageData` 算出来交给它。`shows` 多判一条。用量数据更新、用量日变化时重算这个集合。

关键决定：

- **集合由 `AppRoot` 算好交给表格，`Filter::matches` 不碰用量数据。** `Filter` 是可比较、可克隆的纯数据，`matches` 只看一行。让它持有或引用 `UsageData` 会把一份几千个桶的数据拖进每次比较。集合是 `Arc<HashSet<SharedString>>`，过滤一行多一次哈希查找；5,000 行的过滤仍在 1 ms 内。
- **时间段存在 `Filter` 里，集合存在 delegate 上。** `Filter.active` 变了，`set_filter` 照常报告「过滤变了」，搜索重跑、状态保存的现有流程都不用改；集合只是它的派生物。
- **放宽过滤是一个纯函数**（`Filter::relaxed_for`），返回新的过滤和被放宽项的名字，便于逐项测试，也让通知的文字和实际改动一致。
- **不替用户取消隐藏项目。** 隐藏是用户明确设的；跳转悄悄把它放出来，之后列表里会多出一批他不想看的 session。
- **Active 过滤不进 `state.json`。** 它的时间段是从某一次 Usage 的范围带过来的，重启后那个上下文不在了。`ViewState` 不加字段；`view_state()` 生成快照时不含它。
- **跳转时把 Updated 改成 Any time。** 备选是保留用户原来的 Updated，但「最近更新在今天」和「上周有活动」同时成立的 session 很少，列表多半是空的，看起来像坏了。
- **范围的含义在跳转那一刻定下来。** Usage 的范围是相对的（Today、7 days），带到会话表时换成具体的起止用量日存下来；过了 4:00 不跟着走。标签上显示的也是具体日期。

### 数据变动

无。索引、账本、`state.json` 都不变。

### 逻辑变动

**core `usage.rs`**

- `UsageData::active_sessions(&self, span: Span, model: Option<&str>) -> HashSet<&str>`：有任何一个桶落在这段小时区间里的 session id（主线和 subagent 的桶都算）；给了模型时只算这个模型的桶（两边都按 `pricing::normalize` 比，和 `UsageFilter::model` 相同），其余过滤不看。

**app `model.rs`**

- `ActiveRange { first: NaiveDate, last: NaiveDate }`（起止用量日，都包含），`label()` 返回 `11-12` 或 `11-01 – 11-12`（跨年时带年份）。
- `Filter` 加 `active: Option<ActiveRange>`。`Filter::matches` 不变（活动与否由 delegate 判）。
- `Filter::relaxed_for(&self, row: &SessionRow, calls: CallsInRange) -> (Filter, Vec<&'static str>)`（返工前是 `active_ok: bool`；`CallsInRange` 是 `None` / `OtherModels` / `Picked`，说这一行在 Active 的日子里调用了什么）：逐项检查 `matches` 里的每个条件，不满足的那一项改成不限制，并记下名字（`"Updated"`、`"Project"`、`"Kind"`、`"Model"`、`"Favorites"`、`"Empty sessions"`、`"Active"`）。`hidden_projects` 不在其中，调用方先行判断。

**app `table.rs`**

- `SessionTableDelegate` 加 `active_ids: Option<Arc<HashSet<SharedString>>>`；`set_active_ids(..)` 设置后 `recompute`。`shows(row)` 在 `filter.active.is_some()` 时要求 `active_ids` 含这一行的 id（集合还没给时视为不含）。
- `rows` 里按 id 找一行的方法（`row_by_id(id) -> Option<&Arc<SessionRow>>`；现有的 `position_of` 只在过滤后的 `ix` 里找）。

**app `main.rs` / `toolbar.rs`**

- `AppRoot::sync_active_ids(cx)`：`filter.active` 为 `Some` 时，用 `usage::day_start_ms` 把起止日换成 `Span`，取 `usage.active_sessions(span)`，交给 delegate；为 `None` 时清掉。在 `apply_filter` 之前（过滤的 `active` 变了）、`on_store_events` 收到 `Usage` 之后调用；后一种情况下集合变了还要 `after_rows_changed`，搜索中则 `run_search`。
- `AppRoot::reveal_session(id, window, cx)`：
  1. `row_by_id`；没有 → 通知「这个 session 的记录已被清理，只保留了用量」，返回。
  2. 项目在 `hidden_projects` 里 → 通知，返回。
  3. 搜索中且这一行不在命中里 → 清空搜索框并结束搜索（`clear_search` 的前半段，不动焦点）。
  4. `current_filter(cx).relaxed_for(&row, active_ok)`；有放宽 → `range` 同步成 `RangePreset::Any`（Updated 被放宽时）、`apply_filter`，通知「已清除过滤：…」。
  5. `set_view(Sessions)`；`position_of(id)` → `select_row`；`detail_visible = true`；`focus_table`。
- `AppRoot::show_active_sessions(project: Option<SharedString>, window, cx)`：从 `usage_view` 的当前范围得到起止用量日（`range_span` 的两端换回日期）；新过滤 = 当前过滤，`since_ms` / `until_ms` 清空，`active` 设上，`project` 传了就换；`self.range = RangePreset::Any`；搜索中则清空搜索；`sync_active_ids` → `apply_filter` → `set_view(Sessions)` → `focus_table`。然后算「Usage 这段时间、同样的 Project / Kind / Model 下的 session 数」减去表格显示的行数，大于 0 就通知。
- `AppRoot::clear_active(window, cx)`：过滤的 `active` 置空，`sync_active_ids`，`apply_filter`，`focus_table`。
- `render_toolbar`（Sessions 形态）：`filter.active` 为 `Some` 时在 Updated 后面画标签（`Active {label} ✕`，用现有 `trigger` 的外观加选中色 `style::lit_bg()`），点击调 `clear_active`。
- `usage_view.rs`：session 行加 `on_click` → `reveal_session`；`UsageSnapshot` 的 session 行加 `listed: bool`（会话表里有没有这一行，`refresh_usage` 时查），为假的行短 id 用 `style::muted()`、带 `tooltip`。项目行末尾和 Sessions 卡加 `↗` 按钮，分别调 `show_active_sessions(Some(key))` / `show_active_sessions(None)`；按钮的点击不冒泡到行（行的点击是按项目过滤）。
- `view_state()`：不保存 `active`（`ViewState` 本来就不含它，确认 `AppRoot::new` 恢复出的过滤 `active` 为 `None`）。
- `ASM_USAGE_AUTOPLAY` 加两步（见测试）。

### 实施步骤

- [ ] 1. core `active_sessions`：先写测试，再实现
- [ ] 2. app `model.rs`：`ActiveRange`、`Filter::relaxed_for` 的测试与实现
- [ ] 3. app `table.rs`：`active_ids`、`shows`、`row_by_id` 的测试与实现
- [ ] 4. `keyboard_tests.rs`：跳转、焦点、清除的用例；再实现 `reveal_session`、`show_active_sessions`、`clear_active`、`sync_active_ids`
- [ ] 5. Usage 视图里的点击、`↗`、灰色的行；工具栏的标签
- [ ] 6. autoplay 加步骤，截图验收
- [ ] 7. 文档（`app-architecture.md`：搜索何时重跑的表加「活动集合变化」，焦点表加「从 Usage 跳转 → 表格」；`product-prd.md`）、commit

### 测试与验证

core `crates/core/tests/usage.rs`（验收 5、6）：

- `a_session_is_active_in_a_span_when_any_of_its_calls_falls_inside`：三个 session：调用全在范围内、调用跨范围前后、调用全在范围外；subagent 的调用也算。
- `the_span_ends_are_usage_days`：范围最后一天次日 03:59 的调用算在内，04:00 的不算。

app `model.rs`（验收 2）：

- `relaxing_changes_only_the_filters_that_hide_the_row`：分别构造被 Updated、Project、Kind、Model、只看收藏、空 session、Active 挡住的行，每种只有对应的一项被改，返回的名字正确；同时被两项挡住时两项都改。
- `a_row_that_already_shows_relaxes_nothing`。
- `the_active_label_reads_one_day_or_a_range`。

app `table.rs`（验收 5、6、9）：

- `the_active_filter_keeps_only_sessions_in_the_set_and_stacks_with_the_others`：照 `the_kind_filter_stacks_with_project_time_and_empty_sessions` 的写法。
- `a_session_updated_later_but_active_in_range_is_listed_and_one_updated_in_range_but_idle_is_not`。
- `search_targets_follow_the_active_filter`。
- `without_a_set_the_active_filter_shows_nothing`（集合还没算出来时不误列全部）。

app `keyboard_tests.rs`（无界面窗口，fixture 的调用时间固定，范围用 Custom 指到 fixture 的日期）：

- `revealing_a_session_from_usage_selects_it_and_focuses_the_table`（1）：`ctrl-2` → 调 `reveal_session(S2)` → `view == Sessions`，`selected` 是 S2，表格有焦点，`press("cmd-c")` 后剪贴板是 S2 的 id。
- `revealing_a_filtered_out_session_relaxes_only_what_hides_it`（2）：先把 Updated 设成挡住它的范围、Project 设成它的项目；跳转后 Updated 是 Any，Project 没变，通知里有 `Updated`。
- `revealing_a_session_hidden_by_a_search_clears_the_search`（2）。
- `a_session_without_a_row_stays_in_usage_and_explains`（3）：从表格数据里拿掉一行（或用一个账本里有、索引里没有的 id），`view` 仍是 Usage，有通知。
- `a_session_of_a_hidden_project_is_not_revealed`（4）。
- `jumping_from_a_project_sets_project_any_time_and_active`（5、7）：`show_active_sessions(Some(ALPHA))` 后过滤的三项如预期，表格里的行都在集合里，表格有焦点。
- `the_notice_counts_the_sessions_the_table_cannot_list`（10）：账本里有一个范围内有调用、索引里没有行的 session；跳转后的通知里 N 为 1，等于 Usage 快照的 Sessions 数减去表格的行数；没有这种 session 时不弹通知。
- `clearing_the_active_tag_keeps_the_other_filters_and_the_focus`（8）。
- `searching_under_the_active_filter_and_clearing_it_reruns_the_search`（9）。
- `the_active_filter_is_not_restored_after_a_restart`（11）：`view_state()` 存出的文件再 `ViewState::load`，新建的 `AppRoot` 过滤里 `active` 为 `None`，`range` 为 Any。
- `a_scan_that_adds_calls_in_range_adds_the_session_to_the_list`：追加一条范围内的调用并扫描，行出现。

实机验收（沿用 usage-view 的环境和 `scripts/screenshot-usage.sh`）：

- `ASM_USAGE_AUTOPLAY` 在 `project` 之后加：`jump-project`（对费用最高的项目调 `show_active_sessions`，截会话表：Project、`Active …` 标签、行）→ 回到 Usage → `jump-session`（对费用最高且会话表里有的 session 调 `reveal_session`，截选中和详情）。
- 用 `sqlite3` 在账本副本上查这个项目在这段时间有调用的 session id，和截图里的行逐个对上（验收 5、6）。
- 手动：在一个记录已被清理的 session 上点一次，确认通知和灰色显示（验收 3）；点标签的 ✕（验收 8）。`↗` 和行的点击是鼠标操作，用 cliclick 点一次确认 `↗` 不会同时触发行的过滤。

### 风险与待定

- **`↗` 的点击冒泡**：行上有 `on_click`，按钮在行里。GPUI 里要在按钮的处理函数里 `cx.stop_propagation()`，实现时用上面的 cliclick 检查确认。
- **通知里的 N 可能让人困惑**：Usage 的 Sessions 数包括会话表里没有的、被隐藏项目里的、没有 prompt 的 session。N 的口径定为「Usage 的数 − 列表的行数」，通知的文字如实写「未列出」，不细分原因。实测后如果 N 经常很大（没有 prompt 的 SDK session 多），把「没有 prompt 的 session」也在跳转时改为显示。
- **`selected` 被放宽的过滤带走**：放宽过滤会改变整张表，原来选中的行随之换成目标 session，这是预期的；但 `wanted_id` 的等待逻辑（启动时恢复选中）不能被这次跳转打断。`reveal_session` 里先清 `wanted_id`。
- **Active 的标签占工具栏宽度**：`screenshot-clicks.sh` 的点击坐标是按工具栏按钮的位置写的，标签只在跳转后出现，默认状态下不影响；改了工具栏布局后仍要按 AGENTS.md 校对一次坐标。

## 实现记录（2026-11-16，usage-p4-drill）

实现与验收证据见 `kb/sessions/2026-11-16-usage-p4-drill.md`。和上文不一致的地方（以代码为准）：

- **代码放在新模块 `crates/app/src/drilldown.rs`**：`reveal_session`、`show_active_sessions`、`clear_active`、`sync_active_ids`、`active_ids_in`、通知文字（`GONE_NOTICE`、`HIDDEN_NOTICE`、`unlisted_notice`）和发通知的 `AppRoot::say`（同时打 `notice: …` 到日志，截图脚本据此判定）。`main.rs` / `toolbar.rs` 只加了接线。
- **`ActiveRange::label(year)` 带当前年份参数**，和第 3 期范围标题的做法一样：今年的写 `11-12` / `11-01 – 11-12`；跨年两端都带年份 `2025-12-28 – 2026-01-03`；整段不在今年时 `2025-11-12` / `2025-11-01 – 11-12`。
- **集合在 `apply_filter` 里给**：`active` 变了时 `apply_filter` 自己算集合（`active_ids_in`）并在 `set_filter` 之前交给 delegate，调用方不用先调 `sync_active_ids`；`sync_active_ids` 只用于收到用量之后。
- **用量更新后的刷新走扫描刷新的规矩**：集合真变了才 `relocate_selection(false)`（不把表格滚回选中行），搜索中和整表重读一样走静默的 `refresh_search`，不是上文写的 `after_rows_changed` + `run_search`（AGENTS.md：扫描引起的刷新不滚回选中行、搜索模式下走 `refresh_search`）。
- **`reveal_session` 把 `wanted_id` 设成目标，而不是清掉**：放宽过滤途中表格可能先选中别的行，启动时可能还在等恢复的选中，都让给目标。搜索中又放宽了过滤时，过滤变化引起的重搜记成 `Refresh`（结果攒齐再换），目标在重搜期间一直选中。被清空的搜索在通知里叫 `Search`（`已清除过滤：Search、Updated`）。
- **通知的 N 是集合差，不是减法**：N = Usage 在同样 Project / Kind / Model 下算到、表格却没列出的 session 数。会话表的 Model 按 session 的主模型比，Usage 按每次调用的模型比，开着 Model 过滤时表格里可能有 Usage 没算的行，减法会少算。文字按原因分三种：全是记录已清理的「这段时间还有 N 个 session 的记录已被清理，未列出」；全是被其他过滤挡住的「…被会话表的其他过滤挡住，未列出」；两种都有的「这段时间还有 N 个 session 未列出：K 个的记录已被清理，其余被会话表的其他过滤挡住」。没有 Model 过滤时 N 等于 Usage 的 Sessions 数减去行数（验收 10，导览的 `jump-project` 判定这个等式）。返工后 Active 下 Model 也按调用比（见下节），开着 Model 时减法同样成立；集合差的算法保留，少见的情况下两者仍会不同：会话表 Kind 只勾了 History、关了 CLI 时，记录已删的 CLI session 的 History 行列得出来，Usage 却不算它。
- **Sessions 卡的 `↗` 只在范围内有调用时显示**（`Empty::No`），没有调用、账本为空、账本不可用时不画。
- **autoplay 加在 `ASM_USAGE_VIEW_AUTOPLAY` 的导览里，不是 `ASM_USAGE_AUTOPLAY`**：后者的 gate 驱动改数据的扫描和重建（`make screenshot-usage`），跳转只改视图状态，属于第 3 期的 Usage 视图导览（`make screenshot-usage-view`）。两步放在 `sessions` 之后、`empty` 之前，而不是 `project` 之后：放在中间，Active 过滤会一直带到 `month` / `narrow` / `sessions` 的截图里，前 13 张的含义就变了。状态行加了会话表一侧的字段（`active=`、`updated=`、`rows=`、`selected=`、`detail=`、`focus=`、`usage-sessions=`），`jump-project` 把列出的 id 打成 `jump-rows: …`，供和账本副本逐个对。
- **第 4 步的键盘测试和实现是一起写的**，没有先跑出红。补的证据是变异检查：10 处改动（去掉 `stop_propagation`、收到用量后不同步集合、`apply_filter` 不给集合、不清搜索、不改 Updated、不清 `since/until`、通知不分原因、灰色判定恒真、不查隐藏项目、清标签后不给表格焦点）各自让对应的测试失败（`tmp/2026-11-12-usage-stats/usage-p4-drill/mutations.log`）。
- **测试比上文多四个**：`a_session_left_only_in_history_is_revealed_as_its_history_row`（只剩 History 行的 session 照常选中）；三个鼠标测试 `a_click_on_a_session_line_in_usage_reveals_it`、`a_click_on_a_project_s_arrow_jumps_and_does_not_also_click_its_row`、`the_sessions_card_s_arrow_and_the_tag_s_cross_work_with_the_mouse` 用测试窗口的鼠标点击（`debug_selector` 定位）代替了上文的 cliclick 检查；真点击仍留给实机验收。

## 返工（2026-11-16，usage-p4-drill-r1）

依据审阅 `tmp/2026-11-12-usage-stats/reviews/usage-p4-rv.md` 的 R1、D1 和实机验收 `tmp/2026-11-12-usage-stats/usage-p4-accept/report.md`「发现的问题」1–4。证据和红绿记录在 `tmp/2026-11-12-usage-stats/usage-p4-drill/r1/`，总结在 `kb/sessions/2026-11-16-usage-p4-drill.md` 的 r1 一节。

- **R1，搜索结果还在路上时点 session**：表格在新搜索的第一批结果到达前留着旧命中，原来只看 `hit(id)`，旧命中里有它就保留搜索，新结果到达后把刚选中的它换掉。现在由 `drilldown::keeps_search(settled, hit, truncated, widened)` 决定：搜索已结束、搜索框里的词就是搜过的词（`settled`），它在命中里，而且结果没被截到最新 200 个时才允许在放宽后的过滤上重跑；否则清空搜索（通知里的 `Search`），旧 `seq` 作废，迟到的结果被丢掉。搜索框里打了词还在防抖的情况一样处理（词和 `query` 不同）。选了「清空」而不是「等结果到了再决定」：点击是明确要看它，等待会让它先被选中、再在结果到达时被拿走或出现第二次通知。测试在同一次 update 里发起新搜索、切到 Usage、点它，再手动投递那次搜索的结果，不靠线程竞速。
- **D1，用户决定（brief 决定 20）：下钻时按调用筛**。`Filter.active` 开着时 `Filter::matches` 不看主模型，Model 并进活动集合：`UsageData::active_sessions(span, model)` 只算这个模型的桶，`AppRoot::active_ids_for(filter)` 按过滤的范围和模型取集合；`apply_filter` 在 `active` 或 `model` 变了时重算，所以跳转后再换模型也按调用比，清掉 Active 后回到主模型。`relaxed_for` 的第二个参数从 `active_ok: bool` 换成 `CallsInRange`（`None` / `OtherModels` / `Picked`）：一次调用都没有放宽 Active（之后 Model 按主模型比，不符也放宽），只调用过别的模型放宽 Model。按审阅的 A / B / C 例子写了集合和通知的测试（`under_the_active_filter_the_model_is_the_one_called_in_those_days`）。会话表 Model 下拉里的数字仍是按主模型数的全部 session（它本来就不随 Project、Updated 变），Active 下没有改成按调用数；只在某些天被调用、从来不是主模型的模型在会话表的下拉里选不到，要在 Usage 里选好再跳。
- **验收 1，会话表工具栏在窄窗口下溢出**：选了换行，不收起控件。理由：和 Usage 工具栏一致；所有控件仍是一次点击，不多一层菜单；默认宽度下一行放得下时布局不变（1440px 下各控件位置改前改后逐个对过，`r1/toolbar-1440-before.txt` / `-after.txt`，默认状态完全一致；开着 Active 和正则时搜索框比原来宽 6px，因为去掉的占位元素原来也占一个间距）。Preview 改用 `ml_auto()` 靠右，搜索框 `flex_basis(160).flex_grow(1.).max_w(220)`，先缩到 160 再换行。`screenshot-clicks.sh` 的坐标对默认状态不受影响；但它的 x 坐标早在第 3 期加 `Sessions | Usage` 切换时就已经偏了（例如 Kind 现在在 530–614，脚本点 383），交给协调者。
- **验收 2，700pt 下 Usage 的大数字折行、分隔线落在行末**：窗口窄于 960（`NARROW_W`）时指标卡排成三张加两张，大数字 `whitespace_nowrap`。分隔线的规则不用写死阈值（项目名长短会改变断行处）：`on_children_prepainted` 量出这一帧各子元素的位置，分隔线和前后控件在同一行才画，不画时 `.invisible()` 照样占位，免得宽度变了又能放下一行、来回闪。
- **验收 3**：`index.md` 跨期验收第 3 条按本 spec 的行为改了措辞：删文件之后、重建索引之前点它是选中它的 History 行；Rebuild Index 之后（它不在 `history.jsonl` 里时）才是灰色、点它提示已被清理。
- **验收 4，通知盖住 Sessions 卡的 `↗`**：改了。通知统一放到右下角、状态栏上方（`install_theme` 里 `theme.notification.placement`）。右上角会盖住工具栏右端、Usage 的 Sessions 卡 `↗`，以及跳转之后详情面板标题行的按钮（Copy resume 等），都是下一下最可能点的地方；右下角盖的是内容。测试在通知滑到最终位置后用测试窗口的鼠标点 `↗`。
- **导览加 `sessions-narrow`**：`jump-session` 之后把窗口改到 700px 截会话表（Active 标签还在），状态行加 `toolbar=<行数>/inside|overflow`，脚本判定多于一行且都在窗口里。截图从 17 张变成 18 张（`16-sessions-narrow.png`，之后 `17-empty`、`18-no-usage`）。
- **先红后绿**：R1 两个、D1 三个（含 model.rs 的纯函数测试）在改实现之前跑过红（`r1/red-app.log`）；窄窗口、分隔线、指标卡、通知四个测试也先在旧布局上跑红再改。之后又用 `r1/mutate-r1.py` 把每一项改回旧行为（11 处，含 core 的 `active_sessions` 忽略模型），各自让对应的测试失败（`r1/mutations.log`）。
