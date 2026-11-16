---
created: 2026-11-16
tags:
  - usage
  - menubar
  - app
  - gpui
  - appkit
  - testing
---

# 用量统计第 2 期：常驻菜单栏

## 协调者摘要（usage-p2-resident）

- 结果：完成 menubar-resident 步骤 2–7（步骤 7 只做到脚本和 fixture 副本上的自查）和步骤 9；没有改 core，没有做步骤 8（`make release-local`）和真实数据上的验收。
- 提交：代码、测试、脚本、文档、spec 写回与本总结在同一个提交；hash 见交付回复，或 `git log -1 --format=%H -- kb/sessions/2026-11-16-usage-p2-resident.md`。
- 测试：`cargo test -p agent-session-manager` 177 通过 / 0 失败 / 2 忽略（原 166，新增 11：resident 4、keyboard 7，settings 的两个用例加了字段）；`make test` 550 / 0 / 8。keyboard 用例做过 5 次变异检查（去掉焦点、去掉激活、关窗回调返回 true、不弹说明、`rescan` 不同步菜单），都被抓到。`cargo build --release` 通过。
- 自查：fixture 副本上两次跑通 `make screenshot-resident`（`tmp/2026-11-12-usage-stats/usage-p2-resident/selfcheck2/`、`selfcheck3/`，截图都用 Read 看过）：标题与状态栏一致、关窗后窗口和 Dock 图标消失而进程在、隐藏时 Scan Now 后数字变化、Open Sessions 后 `active=true table-focused=true` 且搜索词 / 选中 / 命中位置不变、经菜单 Quit 退出码 0。
- 决定（都写回了 spec「实现与本 spec 的差异」）：不加 `OpenUsage`；`Resident` 做成 GPUI 全局而不是 `AppRoot` 字段，不加 `cfg(target_os)`；连续两次失败（或从未成功）才显示 `Scan failed`，遇锁跳过也算失败；菜单 Today 行带 `(N unpriced)`；两种回答都写说明已读的标记；autoplay 改成 gate 文件驱动，菜单在固定位置弹出而不是点菜单栏项（Thaw 把它藏在屏幕外）。
- 未验证：真点菜单栏项（Thaw）、真实按键的 `⌘W` / `⌘Q`、红色按钮的真实点击、Finder 重开（要正式包）、Sparkle（要正式包）、窗口隐藏时 App Nap 是否推迟 10 分钟定时器（推测的风险）、`RESIDENT_CLICK=1` 的 cliclick 路径、真实数据。见「给验收的做法」。
- 交给协调者：AGENTS.md 的新环境变量与命令、「关窗不再退出」对开发命令的影响、一条 GPUI 规则；next-up / known-issues 候选。见文末。

## 实现概要

- 依赖（`crates/app/Cargo.toml`）：`objc2-app-kit 0.3.2`（关掉默认 feature，只开 NSApplication、NSStatusBar / NSStatusItem / NSStatusBarButton、NSMenu / NSMenuItem、NSImage、NSWindow、NSScreen 等）、`raw-window-handle 0.6.2`，`objc2-foundation` 加 NSGeometry、NSArray。都是 `Cargo.lock` 里 GPUI 已有的版本，lock 只多两行。
- `resident.rs`（新）：
  - 纯函数：`title`（`$204`、`$1,204`，没有账本是空串）、`menu_today`（`Today  $203.69`，加 ` (N unpriced)`；没有账本 `Today  —`）、`menu_tokens`（`In 492.7M · Out 2.4M`）、`menu_scanned`（`Scanning…` / `Scan failed HH:MM` / `Scanned HH:MM` / `Scanned —`）。`ScanRecord` 记上次成功、上次失败、连续失败次数；`ResidentView` 是推给菜单栏项的全部内容。
  - AppKit：`MenuTarget`（`define_class!`，类名 `AsmMenuTarget`，同时是菜单的 delegate，打印 `menu bar: menu open / closed`），`Resident`（GPUI 全局，持有 item、菜单、三行信息、Scan Now 项、target），`install`、`update`、`hide(window)`（`orderOut:` + Accessory）、`show()`（Regular）、autoplay 用的 `pop_up_later` / `close_menu` / `perform`。
- `settings.rs`：`ViewState.resident_notice_shown`。
- `main.rs`：`CloseWindow` action（`cmd-w`、File > Close Window）；`QuitMode::Explicit`；通道在 `app.run` 前建好，`on_reopen` 和菜单 target 都往里送命令，`AppRoot::listen_to_menu` 收；`on_window_should_close` 回调调 `close_requested` 并返回 false。`AppRoot` 新增 `scan_record`、`window_hidden`、`resident_notice_shown`、`notice_open`、`quitting`、`resident_view`；方法 `sync_resident`（内容变了才推，打印 `menu bar: …`）、`close_requested`、`hide_window`、`show_window`（Regular → `cx.activate(true)` → `activate_window` → 焦点给表格 → `roll_day` → `auto_scan("window shown")`）、`on_resident_command`、`quit`。`ScanDone` 记成功，`ScanSkipped` 和扫描中的 `Failed` 记失败。
- autoplay：`ASM_RESIDENT_AUTOPLAY=<gate 目录>`，命令 `close`、`menu`、`close-menu`、`scan`、`wait-scan`、`open`、`quit`；`ASM_RESIDENT_MENU_AT=x,y`、`ASM_RESIDENT_QUERY=<词>`。
- 脚本：`scripts/screenshot-resident.sh`（新，`make screenshot-resident`）；`scripts/shot-lib.sh`（新，和 `screenshot-usage.sh` 共用）；`scripts/winid.swift` 加 `--screen`。`screenshot-usage.sh` 的 EXIT trap 同样修了（见踩过的坑）。
- 文档：`kb/docs/app-architecture.md`（「窗口隐藏之后」一节、焦点表、视图状态表、相关文件）、`kb/docs/gpui-kit-guide.md`（「菜单栏项与隐藏窗口」一节、测试平台的 prompt / 关窗 / 退出、截图脚本表）、`kb/docs/product-prd.md`（状态表、4.6、4.7、新增 4.10）；spec 加「实现与本 spec 的差异」「实测」，重写「风险与待定」（含 usage-p2-probe 的结论）。

## spec「测试与验证」的对照

| spec 里的用例 | 实际的测试 | 验收项 |
|---|---|---|
| `the_title_is_todays_cost_rounded_to_dollars` | 同名（`resident.rs`） | 1 |
| `the_menu_lines_show_cost_tokens_and_the_last_scan` | 同名，另加 `a_first_scan_that_fails_says_so_at_once`、`the_view_greys_scan_now_while_scanning_and_shows_only_the_icon_without_a_ledger` | 2、9 |
| `a_saved_state_comes_back_unchanged` 加字段、旧文件为 false | 同名加字段；旧文件为 false 的断言加在 `fields_added_later_default_when_an_old_file_lacks_them` | 7 |
| `closing_the_window_hides_it_and_keeps_the_app_running` | 同名：`simulate_close()` 返回 false、隐藏；追加一次调用、推进 600 秒，S1 更新、Today 与菜单第一行一致、窗口仍隐藏 | 3、4 |
| `command_w_closes_like_the_red_button` | 同名，另测搜索框有焦点时的 `cmd-w` | 3 |
| `reopening_restores_the_view_and_gives_the_table_the_focus` | 同名：30 天、按 Prompts 排序、搜 `parser`、命中 1 / 3；在搜索框里关窗；重开后视图状态、搜索词、选中不变，窗口激活，表格有焦点，按 `n` 到 2 / 3 | 5 |
| `the_first_close_asks_once` | 同名（说明开着时再关窗不变；答「好」后隐藏并写入 `state.json`；第二次不弹）；「立即退出」拆成 `quit_now_in_the_first_close_notice_quits`（断言 `quitting` 和标记，测试平台的 quit 是空操作） | 7 |
| `scan_now_from_the_menu_scans_while_hidden` | 同名：隐藏后 Scan Now，同一次 update 里菜单是 `Scanning…`、Scan Now 置灰，之后扫描完成 | 9 |
| （新增） | `the_menu_commands_come_over_a_channel`：命令经通道（菜单 target 和 Finder 重开走的路径）到 `on_resident_command` | 5、6、9 |

## 给验收的做法

准备：`ASM_DB`、`ASM_STATE`、`ASM_CLAUDE_DIR` 指到 `tmp/2026-11-12-usage-stats/accept/` 下的副本（账本跟着 `ASM_DB` 在同一目录）。要用真实索引就把 `index-copy.sqlite` 复制一份过去，不要直接打开它。验收前看一眼 `pgrep -fl agent-session-manager`，结束后用菜单的 Quit 或 `pkill -f agent-session-manager`：关窗口已经不会退出了。

主要工具是 `make screenshot-resident SHOT_DIR=…`（脚本开头有全部选项）。每一步打 `autoplay-resident: <步骤> · <标题> | <三行>`，`shown` 那行还带状态栏的 Today。脚本把应用带到前台两次（启动、Open Sessions），这期间敲的键会进应用窗口，跑之前告诉用户。

1. **菜单栏项和今天的费用**：看日志 `autoplay-resident: shown · $N | Today  $X … · status bar Today $X`，`$N` 应是 `$X` 四舍五入。菜单栏条 `1-shown-menubar.png` 里**看不到**菜单栏项：用户的 Thaw 把新项收在屏幕外。要截到真图，先请用户在 Thaw 里把它拖到可见区（debug 二进制和正式包在 Thaw 里是两个不同的项），再跑脚本。
2. **菜单的顺序和数字**：`3-menu.png`、`5-menu.png` 是同一个 NSMenu 在固定位置弹出的样子（三行信息、分隔线、Open Sessions、Scan Now、分隔线、Quit），数字对日志的 `shown` / `scan` 行。「点菜单栏项弹出」要菜单栏项可见（见 1），点它用 cliclick 鼠标点击（规则允许，弹出层），坐标从截图上读，点之前先确认图标就在那里。
3. **关窗**：`2-hidden-dock.png` 没有图标（`1-shown-dock.png` / `6-open-dock.png` 有；debug 二进制的图标是通用的 `exec`）；`run.log` 里 `window on screen: no`、`process: <pid> …`。autoplay 的 `close` 直接调 `close_requested`（红色按钮回调里调的也是它）；真实按键的 `⌘W` 和真点红色按钮不能自动做（规则不许发按键），由无界面测试覆盖，实机请用户各试一次。
4. **隐藏时数字变化**：`RESIDENT_BEFORE_SCAN` 往副本追加今天的调用（`tmp/2026-11-12-usage-stats/usage-p1-app/append-response.py <副本里的 transcript>`，一次 $0.09，追加 12 次约 $1），然后 Scan Now：`4-scanned-*` 和 `scan` 行的标题、Today 变了。「10 分钟内」那半：`RESIDENT_HOLD=1300`（约 22 分钟）让窗口一直隐藏，脚本打印每次 `auto-scan: timer` 的时刻和当时的菜单内容；定时器从启动起每 600 秒一次，间隔明显超过 10 分钟就是 App Nap（见 spec「风险与待定」），记进 known-issues。
5. **重开**：`open` 行要 `shown=true active=true table-focused=true`；加 `ASM_RESIDENT_QUERY=<词>`，`1-shown-window.png` 与 `6-open-window.png` 的搜索词、过滤、排序、选中、详情位置应相同；`6-open-dock.png` 图标回来。「快捷键立即可用」只能看 `table-focused=true`（加上无界面测试里重开后按 `n` 能跳）；真按键请用户试。
6. **Finder 重开**：只在 `make release-local` 的包上做（步骤 8）：用包跑起来、关窗、请用户在 Finder 里双击包；或在包上设 `ASM_RESIDENT_AUTOPLAY` 跑到 `close` 后让用户双击，日志应出现 `menu bar: OpenSessions` 和 `window: shown`。裸二进制做不了。
7. **第一次关窗的说明**：`RESIDENT_NOTICE=1` 从没显示过说明的状态开始；`close` 步骤会停下等人回答（最多 10 分钟，日志有 `window: first-close notice`）。请用户点「好」，脚本继续；再跑一次（同一个 `ASM_STATE`）应不再弹。点「立即退出」时脚本打印 `the app quit from the first-close notice` 后结束。
8. **退出**：菜单栏菜单的 Quit 由脚本最后一步做（`quit: exit status 0`）；`⌘Q` 和应用菜单的 Quit 请用户在窗口开着、关着（关着时只有菜单栏菜单的 Quit 可用，应用菜单不在）各试，`pgrep` 确认。
9. **Scan Now**：日志里 `menu bar: … | Scanning…` 再到 `Scanned HH:MM`。fixture 的扫描只要几毫秒，截图赶不上 `Scanning…`；要看到置灰，用真实数据副本的冷索引（首次扫描要几秒），在 `shown` 之前弹菜单不现实，所以以日志和 `scan_now_from_the_menu_scans_while_hidden` 为准。`RESIDENT_CLICK=1` 让 cliclick 真点菜单里的 Scan Now（脚本先确认日志里菜单开着才点，坐标 = 菜单左上 + (90, 124)，来自 probe 的实测；没在这台机器上跑过这条路径，第一次跑时看 `3-menu.png` 核对）。
10. **Sparkle**：只在正式包上做（步骤 8）：Check for Updates 弹出 Sparkle 的窗口；窗口关着（Accessory）时它可能出现在别的应用后面，记为已知的表现。

## 交给协调者

- AGENTS.md：
  - 环境变量加 `ASM_RESIDENT_AUTOPLAY`（gate 目录，文件驱动关窗 / 菜单 / Scan Now / 重开 / 退出），命令加 `make screenshot-resident`（和 `screenshot-usage` 一样必须指向副本）。
  - 「手动起的 app 验证后必须 kill」：关窗口不再退出，结束要用菜单栏菜单的 Quit、`⌘Q` 或 `pkill -f agent-session-manager`；`make run` 的 pkill 照旧有效。
  - GPUI 规则可加一条：不要在 GPUI 的 update（action、事件处理）里同步调 `NSWindow performClose:` 或打开 NSMenu，前者让进程 abort（`RefCell already borrowed`），后者是嵌套 run loop；`⌘W` 走 `CloseWindow` action。
  - 结构一节可加 `resident.rs`：菜单栏项与隐藏窗口，命令经通道进 `AppRoot`。
  - 「当前状态」：第 2 期常驻菜单栏已实现、待验收（步骤 8 和实机验收归 usage-p2-accept）。
- spec 的勾选框没动：步骤 2、3、4、5、6、9 完成；步骤 7 的脚本完成、实机验收未做；步骤 1 是 probe 做的，结论已写进「风险与待定」；步骤 8 未做。
- next-up 候选：usage-p2-accept 跑 `RESIDENT_HOLD` 看隐藏时定时扫描的间隔（App Nap）；步骤 8 的正式包上验 6、8、10。
- known-issues 候选（验收证实后再记）：窗口隐藏时 App Nap 推迟定时扫描；窗口隐藏时 Sparkle 的提示可能在别的应用后面；`activateIgnoringOtherApps:` 已弃用（现在可用，失效时退回点击菜单项时 `NSApp.activate()`）。Thaw 收起新菜单栏项是用户环境，不是应用的问题，不记。

## 踩过的坑

- **fixture 的扫描几毫秒就完了**：`run_until_parked` 之后再看已经是 `Scanned`，断言不到 `Scanning…`。在同一次 update 里调 `on_resident_command(ScanNow)` 后立即读 `resident_view`。
- **`screencapture` 偶尔失败**：第一次自查在第二张菜单截图处报 `could not create image from rect`，同一区域稍后再截就成功。`shot-lib.sh` 的两个截图函数失败时重试一次。
- **EXIT trap 让成功的运行以失败结束**：`trap 'kill $PID 2>/dev/null; wait …' EXIT` 在应用已经经 Quit 退出后，`kill` 返回 1，`set -e` 下脚本退出码变成 1，`make` 报 Error 1，而所有步骤其实都过了。改成 `kill $PID 2>/dev/null || true`；`screenshot-usage.sh` 有同样的写法，一并改了。最小复现：旧写法退出码 1，新写法 0。
- **`window.window_handle()` 不是 raw-window-handle 的那个**：GPUI 的 `Window` 有同名固有方法（返回 `AnyWindowHandle`），要写 `HasWindowHandle::window_handle(window)`。测试平台的窗口没有句柄。
- **测试平台的 `quit` 什么都不做**：「立即退出」只能断言应用自己记下的 `quitting`。
- **菜单栏条截不到菜单栏项**：不是代码的问题，Thaw 把新项放进隐藏区。所以 autoplay 不靠点它，而是在固定位置弹出同一个菜单。
