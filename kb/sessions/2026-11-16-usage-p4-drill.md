---
created: 2026-11-16
tags: [usage, filter, navigation, gpui, screenshots]
---

# Usage 第 4 期：从 Usage 回到会话表

## 协调者摘要（usage-p4-drill）

- 结果：spec `kb/plans/2026-11-16-usage-stats/usage-drilldown.md` 步骤 1–7 完成，完成标准全部达到；和 spec 不一致的地方写回了 spec 末尾「实现记录」。
- 提交：本文件与代码、测试、脚本、文档、spec 同一提交，hash 见最终回复。
- 测试：core 382 passed / 0 failed / 7 ignored；app 245 passed / 0 failed / 2 ignored；`make test` 合计 627 passed / 0 failed / 9 ignored。新增 core 2 个、app 25 个（model 3、table 4、drilldown 2、键盘与鼠标 16）。步骤 1–3 先红后绿；步骤 4 的测试和实现一起写，没留下红的记录，改用 10 处变异检查，全部被抓到（`mutations.log`）。
- 截图：`make screenshot-usage-view` 在真实索引和账本的副本上 17 张，全部判定通过、退出码 0；新的 `14-jump-project`（`Active 11-01 – 11-16`，386 行）、`15-jump-session` 看过。跳转列出的行与账本副本逐个对上：本月 vibench 386 / 386、10 月 137 / 137、11-13 34 / 34（其中 3 个最后更新在 11-14 之后），范围内更新过但没有调用的都没列出。
- `make screenshot-usage`（副本，退出码 0）、`make test-screenshot-resident` 仍通过。`make parity` 没跑：core 只加了一个 `UsageData::active_sessions`，`asm` 不调用它，解析和输出都没动。
- 证据目录：`tmp/2026-11-12-usage-stats/usage-p4-drill/`。
- 自己的决定：代码放新模块 `drilldown.rs`；通知的 N 用集合差并按原因分三种说法；用量更新后走 `relocate_selection(false)` + `refresh_search`；跳转两步加在 `ASM_USAGE_VIEW_AUTOPLAY` 导览的 `sessions` 之后；Sessions 卡的 `↗` 只在有调用时显示。
- 未验证：真实鼠标点击（用户在用电脑，没有把应用带到前台点；`↗` 不冒泡、点灰色行、点标签 ✕ 只在测试窗口里用鼠标事件验过）；验收 9 只有测试（要在搜索框里打字）。
- 交给协调者：spec 实施步骤 1–7 的勾选；`AGENTS.md` 补跳转与 Active 过滤；见下文「交给协调者」。

## 做了什么

**core。** `UsageData::active_sessions(span) -> HashSet<&str>`：有任何一个小时桶（主线或 subagent）落在 `[from, until)` 里、且有调用的 session。不看过滤。

**app 的数据与过滤。** `model.rs`：`ActiveRange { first, last }`（起止用量日，都包含）和 `label(year)`；`Filter.active`；`Filter::matches` 拆成几个小判断，行为不变；`Filter::relaxed_for(row, active_ok)` 返回放宽后的过滤和被放宽项的名字（Updated、Project、Kind、Model、Favorites、Empty sessions、Active，隐藏的项目不动）。`table.rs`：delegate 持有 `active_ids`，`passes_filters` 在 `active` 设上时要求行在集合里（集合还没给时一行都不列），`shows` 和 `search_targets` 都经它；`row_by_id` 在全部行里找。

**跳转（`drilldown.rs`，新文件）。** `reveal_session`、`show_active_sessions`、`clear_active`、`sync_active_ids`、通知文字。`toolbar.rs::apply_filter` 在 `active` 变了时先给集合再 `set_filter`；`main.rs::on_store_events` 收到用量后 `sync_active_ids`，集合变了按扫描刷新的规矩走 `relocate_selection(false)`，搜索中走 `refresh_search`。

**界面。** Usage「Most expensive sessions」每行可点（悬停高亮），会话表里没有行的短 id 灰色、悬停说明；「By project」每行末尾 `↗`（点击 `stop_propagation`，行的其余部分仍是按项目过滤）；Sessions 卡右上角 `↗`；两块的副标题写明怎么点。会话表工具栏 Updated 后面的 `Active … ✕` 标签（`lit_bg`，点了清掉）。新图标 `ArrowUpRight`。

**autoplay 与脚本。** `ASM_USAGE_VIEW_AUTOPLAY` 导览加 `jump-project`、`jump-session` 两步（放在 `sessions` 之后、`empty` 之前），状态行加 `active= updated= rows= selected= detail= focus= usage-sessions=`；`jump-project` 打 `jump-rows: <ids>`。`scripts/screenshot-usage-view.sh` 加两步的判定：Active 和 Project 设上、Updated any、焦点在表格、行数加通知里的 N 等于 Usage 的 Sessions 数；选中的就是 `reveal:` 那个、详情开着、焦点在表格。`AppRoot::say` 发通知时同时打 `notice: …`，`clear_active` 打 `active: cleared`，`show_active_sessions` 打 `active: <起> – <止> · project <key>`，`reveal_session` 打 `reveal: <短 id>`。

**文档。** `app-architecture.md`（「从 Usage 回到会话表」一节、重搜表加「活动集合变化」、焦点表加「从 Usage 跳转」、不保存表加 Active、相关文件）、`product-prd.md`（状态表与 4.11）、`gpui-kit-guide.md`（导览 17 张与新字段、可点的行里放按钮要 `stop_propagation`、测试窗口里的鼠标点击、按键后断言焦点的坑、WAL 不空时怎么复制账本）、`Makefile` 的 help、spec 的实现记录。

## 测试对应

| 验收 | 测试 | 实机证据 |
|---|---|---|
| 1 选中、滚到、详情、焦点 | `revealing_a_session_from_usage_selects_it_and_focuses_the_table`（含 `cmd-c` 复制到它的 id）、`a_click_on_a_session_line_in_usage_reveals_it`、`a_session_left_only_in_history_is_revealed_as_its_history_row` | 导览 `15-jump-session.png`（`selected=48f4bebf detail=yes focus=table`） |
| 2 只放宽挡住的 | `relaxing_changes_only_the_filters_that_hide_the_row`、`a_row_that_already_shows_relaxes_nothing`、`revealing_a_filtered_out_session_relaxes_only_what_hides_it`、`revealing_a_session_hidden_by_a_search_clears_the_search` | — |
| 3 没有行：留在 Usage、灰色 | `a_session_without_a_row_stays_in_usage_and_explains` | `gone/usage-0916.png`：09-16 前 6 里 4 个灰色短 id，正是账本里有、索引里没有的 4 个（`gone-candidates.txt`） |
| 4 项目被隐藏 | `a_session_of_a_hidden_project_is_not_revealed` | — |
| 5 项目的 `↗` | `jumping_from_a_project_sets_project_any_time_and_active`、`a_click_on_a_project_s_arrow_jumps_and_does_not_also_click_its_row` | `14-jump-project.png` + `jump-project-vs-ledger.txt`（386 / 386） |
| 6 按调用而不是按更新时间 | `a_session_updated_later_but_active_in_range_is_listed_and_one_updated_in_range_but_idle_is_not`、core 两个 | `oneday/`（3 个最后更新在范围之后的被列出）、`lastmonth/`（8 个范围内更新但没调用的没列出） |
| 7 Sessions 卡的 `↗` | `jumping_from_the_sessions_card_keeps_project_kind_and_model`、`the_sessions_card_s_arrow_and_the_tag_s_cross_work_with_the_mouse` | — |
| 8 标签 ✕ | `clearing_the_active_tag_keeps_the_other_filters_and_the_focus`、上面的鼠标测试 | — |
| 9 Active 下搜索、清掉后重搜 | `search_targets_follow_the_active_filter`、`searching_under_the_active_filter_and_clearing_it_reruns_the_search` | — |
| 10 通知的 N | `the_notice_says_why_sessions_are_not_listed`、`the_notice_counts_the_sessions_the_table_cannot_list` | 导览 `jump-project` 判定 rows + N = Usage Sessions（这几次 N 都是 0） |
| 11 重启后没有 Active | `the_active_filter_is_not_restored_after_a_restart` | 导览结束时的 `state-195007.json`：`range: Any`，没有 Active 字段 |
| 扫描后列表跟着变 | `a_scan_that_adds_calls_in_range_adds_the_session_to_the_list`、`without_a_set_the_active_filter_shows_nothing` | — |

## 给验收的做法

**准备副本。** 先 `pgrep -fl agent-session-manager` 确认没有应用在写，再把真实数据目录的 `index.sqlite`、`usage-ledger.sqlite` 连同各自的 `-wal`、`-shm` 用 `cp -c` 复制到一个新目录（账本的 WAL 不空时不能只用 `immutable=1` 备份，会漏掉 WAL 里的数据）。`ASM_DB=<目录>/index.sqlite`，`ASM_STATE` 指到同目录的新文件；`ASM_CLAUDE_DIR` 可以用真实的 `~/.claude`（只读）。**直接启动二进制会调 `cx.activate(true)`，可能把窗口带到前台**：启动前也要按 brief 确认用户空闲 ≥ 25 秒。

**用 `state.json` 预设，省掉点下拉菜单。** 启动前写好 `ASM_STATE` 指向的文件，字段名见 `settings.rs::ViewState`，例如 `{"view": "Usage", "usage_range": "Custom", "usage_custom_range": ["2026-09-16", "2026-09-16"], "range": "Today", "kinds": {"cli": false, "sdk": true, "bg": true, "history": true}, "hidden_projects": ["<key>"]}`。`usage_range` 取 `Today` / `Yesterday` / `Days7` / `Days30` / `ThisMonth` / `LastMonth` / `Custom`，会话表的 `range` 取 `Today` / `Days3` / `Days7` / `Days30` / `Any` / `ThisMonth` / `LastMonth` / `Custom`。

**不用点的部分。**

- 导览：`ASM_DB=… SHOT_DIR=… make screenshot-usage-view`，看退出码和 `all checks passed`，`14-jump-project.png`、`15-jump-session.png` 用 Read 打开看。只跑跳转：`ASM_USAGE_VIEW_STEPS=jump-project,jump-session`，配合 `state.json` 的 `usage_range` 换范围（`jump-project` 取 Usage 当前范围里费用最高的项目）。
- 和账本对行（验收 5、6）：`python3 -I tmp/2026-11-12-usage-stats/usage-p4-drill/compare-jump-rows.py <数据目录> <app 日志> <项目 key> <起始日> <结束日>`。项目 key 和起止日在状态行的 `project=`、`active=` 里。注意副本被后来的启动扫过新文件后再对，会多出跳转之后才开始调用的 session（本次就有一个 19:53 才开始的）。
- 找「会话表里没有」的 session（验收 3、10）：`gone-candidates.txt` 的查询（账本 `sessions` 里不在索引 `sessions` 里的 id，按天粗算排名）。今天的副本里 09-16 的前 6 有 4 个。

**要真点击的部分**（每次点击前照 brief：`winid.swift --at` 确认点击处是本应用的窗口、前台是它、指针停 ≥ 0.8 秒）：

1. 验收 1：Usage 里点一个蓝色短 id 的行 → 会话表选中它、详情显示它；日志 `reveal: <短 id>`。焦点在表格只能靠用户按方向键看，或看导览状态行的 `focus=table`。
2. 验收 2：`state.json` 预设 `range: "Today"` 和关掉某个 Kind，Usage 选 30 天，点一个被挡住的 session → 日志 `notice: 已清除过滤：Updated、Kind`，工具栏 Updated 是 Any time、Kind 勾上了它的类型，其他不变。
3. 验收 3：预设 09-16（见上），点一个灰色短 id → 视图不变，日志 `notice: 这个 session 的记录已被清理，只保留了用量`；悬停灰色短 id 看提示。
4. 验收 4：预设 `hidden_projects` 含某个项目，在它有 session 进前 6 的范围里点它 → 日志 `notice: 这个 session 的项目在会话表里被隐藏…`，`state.json` 的 `hidden_projects` 不变。
5. `↗` 不冒泡（spec 风险第 1 条）：点项目行的 `↗` → 日志有 `active: … · project <key>`、`view: Sessions`；点同一行的其余部分 → 只在 Usage 里按项目过滤（Project 下拉变了，视图不变）。
6. 验收 7、10：点 Sessions 卡右上角的 `↗` → 日志 `active: … · project all`（或当前项目）；09-16 这天会有 `notice: 这段时间还有 N 个 session …`，N 应等于卡上的 Sessions 数减状态栏的显示行数（没有 Model 过滤时）。
7. 验收 8：点工具栏 `Active … ✕` → 日志 `active: cleared`，标签消失，Project / Kind / Model 不变。
8. 验收 11：跳转后经菜单 Quit（或 `pkill -f agent-session-manager`），同一个 `ASM_STATE` 再启动 → 没有 Active 标签，Updated 是 Any time。
9. 验收 9 要在搜索框里打字，脚本发不了按键；只有测试覆盖，要实测得请用户自己打。

## 交给协调者

- spec 实施步骤 1–7 的勾选（brief 不让我改）。
- `AGENTS.md`：「当前状态」写第 4 期完成；「各功能要先知道的事」的 Usage 视图一条补上跳转（`drilldown.rs`，Active 过滤的集合在表格 delegate 上、不进 `state.json`，Usage 里的行内按钮要 `stop_propagation`）；截图命令一节里 `screenshot-usage-view` 的说明可以带上两步跳转。
- `screenshot-clicks.sh` 的坐标不受影响：`Active …` 标签只在跳转后出现，默认状态下工具栏不变（`usage-run/screenshots/3-rebuilt.png` 的工具栏和原来一样）。
- 可考虑记进 `kb/known-issues.md`（测试层面，不影响应用）：键盘测试里按键后再程序化移焦点，焦点偶尔回到搜索框，复不复现随编译结果变，原因没查到；已在 `gpui-kit-guide.md` 的测试表里写了绕法。
- 一次越界：19:57 为截 09-16 的灰色行直接启动了一次应用，没先查空闲时间，应用自己跑到了前台，约 3 秒后杀掉。之后的两次启动都先等到空闲 ≥ 25 秒（54 秒、74 秒）。

## 踩过的坑

- **fixture 的文字不像 grep 出来的那样能被搜到。** S3 的原始 JSON 里有 `parser`，但 FTS 只索引 user / assistant 的文本，它命中不了；四个 fixture session 的正文没有共同的词。要两个 session 都命中的查询，用正则 `parser|hello`（S1、S2）。
- **测试里的自动扫描会被节流吞掉。** `auto_scan("test")` 在启动扫描刚结束时返回 false（30 秒节流），`wait_until` 就一直等不到。先 `cx.executor().advance_clock(AUTO_SCAN_MIN_GAP)`，再断言 `auto_scan` 返回 true。
- **Updated 预设挡不住 fixture 的所有行。** 想用 `Today` 挡住 alpha 项目 9 月的 session，结果还剩 2 行（没细查是哪两行、更新时间从哪来）。改用 2000 年的 Custom 区间，一行都不剩。
- **按键之后断言焦点不稳。** `press("cmd-f")` 后程序化 `focus_table`，焦点有时又回到搜索框；同一个用例单跑连败 6 次，加一行 `run_until_parked` 连过 3 次，再加一行注释又连败。`focus_search` 只被调用了一次，回去的路径没找到。测试只要「焦点先在别处」，就直接调 `focus_search`，连跑 10 次、全量 3 次都过，变异检查也照样抓得到。
- **截图脚本里 `grep` 没匹配会让 `set -euo pipefail` 直接退出。** 没有通知时 `grep … | tail | sed` 返回 1，脚本在 `jump-project` 判定处无声退出（make 报 Error 1）。加 `|| true`。
- **直接启动应用会抢前台。** `main.rs` 启动时 `cx.activate(true)`；导览脚本那几次窗口没到前台，手动启动那次到了，是否成功看系统当时的状态。所以任何启动都要按「激活前确认空闲」处理。

## 相关文档

- spec：`kb/plans/2026-11-16-usage-stats/usage-drilldown.md`（末尾「实现记录」）
- `kb/docs/app-architecture.md`「从 Usage 回到会话表」、`kb/docs/gpui-kit-guide.md`「测试」「脚本化交互与截图」
- 证据：`tmp/2026-11-12-usage-stats/usage-p4-drill/`：`app-tests.log`、`core-tests.log`、`make-test.log`、`mutations.log`、`tour.log` 与 `screenshots/`（01–17、`app-195007.log`）、`jump-project-vs-ledger.txt`、`oneday/`、`lastmonth/`、`gone/`、`gone-candidates.txt`、`usage-run/`（`make screenshot-usage`）、`test-screenshot-resident.log`、`compare-jump-rows.py`。`data/` 和 `usage-run/claude/` 是 APFS 克隆的副本，验收完可以删
