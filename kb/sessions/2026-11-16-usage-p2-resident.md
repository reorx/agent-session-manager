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

（R1 改写：脚本有了判定和新选项，加了用户手动检查清单。）

准备：`ASM_DB`、`ASM_STATE`、`ASM_CLAUDE_DIR` 指到 `tmp/2026-11-12-usage-stats/accept/` 下的副本（账本跟着 `ASM_DB` 在同一目录）。要用真实索引就把 `index-copy.sqlite` 复制一份过去，不要直接打开它。验收前看一眼 `pgrep -fl agent-session-manager`，结束后用菜单的 Quit 或 `pkill -f agent-session-manager`：关窗口已经不会退出了。

主要工具是 `make screenshot-resident SHOT_DIR=…`（脚本开头有全部选项）。每一步打 `autoplay-resident: <步骤> · <标题> | <三行>`，`shown` 那行还带状态栏的 Today。**退出码现在可信**：每一步都有判定，不过的项打印 `FAILED: …`，最后列出全部并退出 1；全过才打 `all checks passed` 退出 0。判定本身的测试是 `make test-screenshot-resident`（不起 GUI）。脚本按进程 ID 找窗口，二进制可以是 .app 里的那个，不再需要 `WIN_OWNER`。脚本把应用带到前台两次（启动、Open Sessions），这期间敲的键会进应用窗口，跑之前告诉用户。

推荐的一次跑法（R1 自查就是这样跑的，`tmp/2026-11-12-usage-stats/usage-p2-resident/r1/c1/`）：

```bash
A=$PWD/tmp/2026-11-12-usage-stats/accept/<run>   # 先 cp -R crates/core/tests/fixtures/claude $A/claude
F=$A/claude/projects/-Users-me-Code-alpha/11111111-aaaa-4bbb-8ccc-000000000001.jsonl
ASM_DB=$A/index.sqlite ASM_CLAUDE_DIR=$A/claude ASM_RESIDENT_QUERY=parser RESIDENT_FRONT_APP=Finder RESIDENT_CLICK=1 \
  RESIDENT_BEFORE_SCAN="for i in \$(seq 12); do python3 $PWD/tmp/2026-11-12-usage-stats/usage-p1-app/append-response.py '$F' > /dev/null; done" \
  make screenshot-resident SHOT_DIR=$A/screenshots
```

1. **菜单栏项和今天的费用**：脚本判定标题等于状态栏 Today 取整（`shown` 行）。菜单栏条截图里**看不到**菜单栏项：用户的 Thaw 把新项收在屏幕外，目视要用户先在 Thaw 里拖出来（手动清单 A）。
2. **菜单的顺序和数字**：`3-menu.png`、`5-menu.png` 现在按窗口 ID 截的是菜单窗口本身（R1 截到了，顺序是三行信息、分隔线、Open Sessions、Scan Now、分隔线、Quit）。前台应用很忙（游戏）时程序化弹出的菜单会自己关，脚本判不过并报出前台应用；这时换个安静的前台应用重跑，或用手动清单 A。
3. **关窗**：判定 `hidden=true`、窗口不在屏幕上、进程在；`2-hidden-dock.png` 没有图标。真实的 `⌘W` 和红色按钮见手动清单 D、E。
4. **隐藏时数字变化**：`RESIDENT_BEFORE_SCAN` 追加今天的调用，判定 Scan Now 后 Today 变了。定时器那半用 `RESIDENT_HOLD=1300`：两次 `auto-scan: timer` 间隔超过 660 秒判不过。验收已测过屏幕亮着时间隔正好 600 秒；熄屏（不挂 `caffeinate`）没测。
5. **重开**：`RESIDENT_CLICK=1` 时 cliclick 真点 Open Sessions（点之前用 `winid --at` 确认点击处最上层是本进程的菜单，否则不点、判不过）；`RESIDENT_FRONT_APP=Finder` 让 Finder 先在前台。判定 `shown=true active=true table-focused=true`。不加 `RESIDENT_CLICK` 是程序化选择，不算用户点击，前台是游戏时会 `active=false`。`ASM_RESIDENT_QUERY` 让 `1-shown-window.png` 和 `6-open-window.png` 可以对比搜索词、选中、命中位置。前台是游戏时的点击见手动清单 B。
6. **Finder 重开**：`RESIDENT_REOPEN=<.app>`，二进制用那个 .app 里的（`<.app>/Contents/MacOS/agent-session-manager`，直接跑 `scripts/screenshot-resident.sh <二进制> <输出目录>`）。脚本先确认 LaunchServices 认得这个进程，`open` 之后若冒出第二个实例（它没有环境变量，会打开真实数据）立即杀掉并判不过。包可以是 `make build-release` 的产物，或者像 R1 那样把 debug 二进制打成临时 bundle：`SPARKLE_DIR=<假框架目录> scripts/release/bundle-app.sh "$(make -s bin)" <目录>/Agent\ Session\ Manager.app 0.0.0 0`，假框架是 `Sparkle.framework/Versions/B/Sparkle` 一个普通文件，加载失败后更新器关闭，不会弹 Sparkle 的窗口。在 Finder 里真双击见手动清单 C。
7. **第一次关窗的说明**：`RESIDENT_NOTICE=1`，`close` 步骤停下等人回答（最多 10 分钟）。只有日志有 `window: first-close notice · 立即退出` 且退出码为 0，脚本才把进程退出判为通过。点按钮见手动清单 D。
8. **退出**：脚本最后经菜单栏菜单的 Quit，判定进程 5 秒内退出且退出码为 0。`⌘Q` 和应用菜单的 Quit 见手动清单 F。
9. **Scan Now**：日志 `Scanning…` 到 `Scanned HH:MM`。fixture 扫描几毫秒，截图赶不上置灰，以日志和 `scan_now_from_the_menu_scans_while_hidden` 为准。
10. **Sparkle**：只在带 Sparkle 的包上，见手动清单 G。

### 用户手动检查清单

这些要真实的点击或按键，脚本不做（项目规则不许发按键，cliclick 只点本应用的菜单）。用哪个构建：

- **debug 二进制**（`make bin` 打印路径）：没有 Sparkle，Dock 图标是通用的 `exec`。必须指向副本：`ASM_DB=<副本>/index.sqlite ASM_STATE=<副本>/state.json ASM_CLAUDE_DIR=<副本>/claude "$(make -s bin)"`。
- **包**：`make build-release` 产出 `~/Library/Caches/cargo-target/release/Agent Session Manager.app`（已签名、未公证，本机可直接开）；`make release-local` 现在应该能公证（用户已签开发者协议），但出包不在本任务范围。包同样指向副本：`open -n "<包>" --env ASM_DB=… --env ASM_STATE=… --env ASM_CLAUDE_DIR=…`。

| # | 检查什么 | 怎么做 | 看到什么算过 | 构建 |
|---|---|---|---|---|
| A | 菜单栏项 | 应用开着时打开 Thaw 的设置，把「agent-session-manager」（debug）或「Agent Session Manager」（包）从隐藏区拖到可见区；然后点它 | 菜单栏出现柱状图图标和 `$N`，等于窗口状态栏 `Today $X` 四舍五入；点开的菜单从上到下是 `Today  $X`、`In … · Out …`、`Scanned HH:MM`、分隔线、Open Sessions、Scan Now、分隔线、Quit Agent Session Manager | 任一 |
| B | 别的应用在前台时点 Open Sessions | `⌘W` 关窗；把另一个应用带到前台：先 Finder，再一个全屏游戏（验收出问题的情形）；点菜单栏项 → Open Sessions | 窗口到最前，红黄绿按钮是彩色（是 key window）；左上角菜单栏是本应用的菜单；Dock 图标回来；不点窗口直接按 `↓`，表格选中行移动 | 任一，游戏那次最好用包 |
| C | Finder 双击重开 | 窗口关着时，在 Finder 里双击包 | 同 B；不会多出第二个进程（`pgrep -fl "Agent Session Manager"` 只有一个） | 包 |
| D | 第一次 `⌘W` 的对话框 | 用一个新的 `ASM_STATE` 路径启动；按 `⌘W`。先点「好」；再从菜单 Open Sessions 重开，再按 `⌘W`。另起一次新的 `ASM_STATE`，按 `⌘W` 后点「立即退出」 | 弹出窗口上的对话框「Agent Session Manager 会留在菜单栏继续记录用量。」/「要完全退出请按 ⌘Q。」；「好」后窗口消失、Dock 图标消失、进程还在；第二次 `⌘W` 不再弹；「立即退出」后进程没了 | 任一 |
| E | 红色按钮 | 窗口开着时点红色按钮 | 同 `⌘W`：窗口消失、Dock 图标消失，`pgrep` 还有进程，菜单栏项还在 | 任一 |
| F | 三处退出 | 窗口开着时按 `⌘Q`；重启后点应用菜单的 Quit Agent Session Manager；重启后关窗，再点菜单栏菜单的 Quit | 每次 `pgrep -fl agent-session-manager` 为空。窗口关着时应用菜单不在（菜单栏是别的应用的），只有菜单栏菜单的 Quit | 任一 |
| G | Check for Updates | 包里打开应用菜单的 Check for Updates…；再关窗，等或触发 Sparkle 的提示 | 弹出 Sparkle 的窗口（本地包版本 0.0.0，会提示线上版本）；Install and Relaunch 会退出并重启（会把线上版本装到这个包的位置，测完删掉这个包）。窗口关着时提示可能在别的应用后面，记为已知表现 | 只能用包 |

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

## 协调者摘要（usage-p2-resident-r1）

- 结果：返工任务 `tasks/usage-p2-resident-r1.md` 的 1–5 条都做了；Open Usage 仍不在本期。用户中途允许跑 GUI，第 1 条在 Finder 前台时实机自查过。
- 提交：代码、测试、脚本、spec、文档与本节在同一个提交；hash 见交付回复，或 `git log -1 --format=%H -- kb/sessions/2026-11-16-usage-p2-resident.md`。
- 测试：`cargo test -p agent-session-manager` 179 / 0 / 2（新增 2）；`make test` 552 / 0 / 8；`make test-screenshot-resident` 13 / 13，3 次变异（Quit 退出码不查、说明框退出不查标记、窗口还在屏幕上不判不过）各被一个用例抓到；四个 shell 脚本 `bash -n` 通过。
- 第 1 条（前台）：Open Sessions 菜单项的回调和 `on_reopen` 在 AppKit 回调里当场切 Regular 并 `NSApp.activate()`（`resident::bring_forward`），之后照旧 `cx.activate(true)`。实机（`tmp/2026-11-12-usage-stats/usage-p2-resident/r1/`，Finder 在前台）：c1 cliclick 真点 Open Sessions、c2 程序化选择、c3 `open` 再次打开临时 bundle，都是 `active=true`，菜单栏换回本应用的菜单，Dock 图标回来；三次 `all checks passed`，结束后没有残留进程。
- 第 2 条（判定）：每一步有成功条件，不过的项最后汇总并退出 1，没法继续的立即退出 1；Quit 和说明框退出都查退出码，说明框还要 `window: first-close notice · 立即退出` 标记。
- 第 3 条：按进程 ID 找窗口（`winid - --pid N`），.app 不再需要 `WIN_OWNER`；`screenshot-usage.sh` 同样改了。
- 第 4 条（菜单截图）：原因是前台是游戏时，程序化弹出的菜单不到一秒就自己关了，截图区域本身没错（坐标都是 1884,30，按主屏算）。现在按窗口 ID 截菜单窗口本身（`winid --menu`），截之前和之后都确认菜单开着；R1 在 Finder 前台时截到了完整的菜单。
- 第 5 条：store 线程先关请求通道再报 `Failed`；`Store::request` 返回是否送达；送不出去时扫描当场算失败（`Scan failed HH:MM`，状态栏保留打不开的原因）。先写的两个测试先失败（app 用例停在 `scan_in_flight`），改完通过。
- 未验证：前台是全屏游戏时真点 Open Sessions（这次没有游戏可开，验收出问题的正是这种情形）；Finder 里真双击；对话框按钮、红色按钮、`⌘W` / `⌘Q` / 应用菜单 Quit 的真实操作；Sparkle；熄屏时的定时器。都在「用户手动检查清单」。
- 交给协调者：见下文「R1 交给协调者」。

### R1 的改动

- `store.rs`：`Store::request` 返回 `bool`；`run` 打不开索引时先 `drop(requests)` 再发 `Failed`。测试 `requests_after_the_index_failed_to_open_are_refused`。
- `main.rs`：`request_scan`（`rescan`、`auto_scan` 共用，送不出去就按失败处理，`rescan` 只在送达时把状态栏设为 Loading）、常量 `STORE_GONE`；`on_reopen` 先 `resident::bring_forward()`；说明框的回答打日志 `window: first-close notice · <按钮>`；autoplay 加 `wait-open`。测试 `scans_fail_at_once_when_the_index_never_opened`（无界面；`keyboard_tests.rs` 拆出不等启动扫描的 `start`）。
- `resident.rs`：`bring_forward`、`set_policy`（已是目标 policy 就不再设，原来会打出误导的 `activation policy … refused`）；`openSessions:` 先 `bring_forward` 再发命令；菜单 delegate 记打开的时刻，`menu bar: menu closed (open N s)`。
- `scripts/winid.swift`：`-` 加 `--pid` 按进程匹配；普通窗口只认 layer 0（不会把同进程的菜单当成主窗口）；`--menu --pid N`；`--at X,Y`。
- `scripts/screenshot-resident.sh`：判定与退出码、`RESIDENT_FRONT_APP`、`RESIDENT_CLICK=1` 也点 Open Sessions、`RESIDENT_REOPEN`、`RESIDENT_SETTLE`、`RESIDENT_HOLD` 的间隔判定、按窗口 ID 截菜单、点击前的 `winid --at` 确认。新 `scripts/test-screenshot-resident.sh` 与 `make test-screenshot-resident`。`shot-lib.sh` 认 `SHOT_WINID`。
- 文档：`kb/docs/app-architecture.md`（重开的两种激活与实测、store 线程不在时的扫描）、`kb/docs/gpui-kit-guide.md`（菜单栏表格、脚本的判定 / 点击确认 / 菜单自己关、`winid` 的新用法）；spec「风险与待定」改写激活一条，加 Finder 重开的先后、程序化菜单自己关、store 线程退出、App Nap 的验收结果。

### R1 交给协调者

- AGENTS.md：命令加 `make test-screenshot-resident`；`screenshot-resident` 的退出码现在可信，可以写「验收以退出码和日志为准」。
- known-issues 候选：前台是全屏游戏时，没有点击的激活会被拒（autoplay 用的路径）；用户点击的路径已改，待手动清单 B 确认后再决定记不记。审阅提到的「store 线程退出后菜单停在 Scanning」已修，不用记。
- next-up 候选：用户做手动检查清单 A–G（尤其 B 的游戏情形和 G）；`make release-local` 现在应该能公证，出包和标准 10 归验收或协调者。
- 审阅还建议补换日时菜单内容的断言、重开前后滚动位置的断言：这次没做（非阻断），可留给以后。

### R1 踩过的坑

- **macOS 的 `bash` 是 3.2**：脚本不能用空数组展开（`set -u` 下报错）、关联数组等 4.x 写法；不过的项用字符串累加。
- **`wait` 只能在当前 shell 里调一次**：退出码要用 `reap`（`wait "$PID" || APP_RC=$?`）在当前 shell 取，放进 `$(…)` 就是子 shell，取不到子进程。
- **函数在 `||` 里调用时 `set -e` 失效**：`step close || rc=$?` 里 `step` 的失败分支必须自己显式处理，不能指望 `-e`。
- **`setActivationPolicy:` 设成已有的值返回 NO**，会被当成「被拒绝」打日志。
- **测试替身要防止碰到真工具**：`test-screenshot-resident.sh` 用 `env -i` 和只含替身目录 + 系统目录的 PATH 跑，`cliclick` 只有替身（真的在 `/opt/homebrew/bin`，不在 PATH 里），替身被调用就算用例失败。
