---
created: 2026-11-16
tags:
  - menubar
  - status-item
  - window
  - appkit
  - agent-session-manager
  - plan
---

# 常驻菜单栏：关掉窗口不退出，后台照常记账

总 plan 是 [index.md](index.md)。这是第 2 期，依赖 [usage-ledger](usage-ledger.md)（账本、自动扫描、内存里的用量数据）。

## PRD

### 背景

账本只在应用读到用量时才记。应用关着的时候不扫描，如果一个 session 的记录文件在两次打开之间被删掉，它的用量就从没进过账本。用户的要求是：「软件打开的时候定期扫描。软件打开后，常驻 menubar，关闭窗口不影响后台运行。」

现在关掉窗口的效果很别扭：应用没有退出，Dock 图标还在，但后台线程已经停了，点 Dock 图标也打不开窗口，只能 `⌘Q` 再重新启动。

### 目标

应用启动后在菜单栏放一个图标和今天的费用。关掉窗口后应用留在菜单栏，照常每 10 分钟扫描一次；从菜单栏可以重开窗口、立即扫描、退出。

### 功能与交互

以下各项的选择是 2026-11-16 和用户逐条确认的。

**菜单栏项**

- 应用启动后菜单栏右侧出现一项：一个单色小图标，后面跟今天的费用，取整到美元，如 `$204`；满一千显示千分位，如 `$1,204`。今天的定义同状态栏（4:00 起，全部调用）。
- 每次扫描结束后更新；过了 4:00 归零重计。账本不可用时只显示图标。
- 点它弹出菜单，不直接开窗口。

**菜单**

从上到下：

| 项 | 行为 |
|---|---|
| `Today  $203.69` | 只读。今天的费用，精确到分 |
| `In 492.7M · Out 2.4M` | 只读。今天的 In 和 Out |
| `Scanned 15:53` | 只读。上一次扫描结束的时刻；正在扫描时是 `Scanning…` |
| 分隔线 | |
| `Open Sessions` | 显示窗口并切到会话表；窗口已经开着就把它带到最前 |
| `Open Usage` | 显示窗口并切到 Usage 视图。这一项在 [usage-view](usage-view.md) 合并后才有 |
| `Scan Now` | 立即扫描一次；正在扫描时置灰 |
| 分隔线 | |
| `Quit Agent Session Manager` | 退出应用 |

**关窗与重开**

- 红色关闭按钮和 `⌘W` 只关掉窗口，应用留在菜单栏继续运行。File 菜单（新加）里有 Close Window `⌘W`。
- 窗口关掉后 Dock 图标消失，应用也不出现在 `⌘Tab` 里。重开窗口时 Dock 图标恢复。
- 重开的方式：菜单栏菜单的 Open Sessions / Open Usage；在 Finder 或 Spotlight 里再次打开应用。
- 重开后的窗口和关掉前一样：同一个视图、过滤、排序、选中的 session、搜索词、滚动位置都在。关掉期间扫到的变化已经在表格里。
- 第一次关窗时弹一个系统对话框说明：「Agent Session Manager 会留在菜单栏继续记录用量。要完全退出请按 ⌘Q。」按钮是「好」和「立即退出」。只弹这一次。

**退出**

- `⌘Q`、应用菜单的 Quit、菜单栏菜单的 Quit 都真正退出应用，后台扫描随之停止。
- 应用更新（Sparkle 的 Install and Relaunch）、注销、关机照常退出，不受「关窗不退出」影响。

**后台扫描**

- 窗口关掉后，自动扫描照 [usage-ledger](usage-ledger.md) 的节奏继续：每 10 分钟一次。窗口重开时立即扫一次。
- 窗口关着时扫描出错（例如 `asm` 正占着索引）不弹任何东西，下一次再试；连续失败时菜单的第三行显示 `Scan failed 15:53`。

**更新提示**

- Sparkle 每天检查一次更新。窗口关着时发现新版本，它的提示窗口会自己出现，这是可以接受的；不做额外处理。

### 不做的事

- 不做登录时自动启动（2026-11-16 确认，以后和设置页一起做）。重启电脑后要自己打开一次应用。
- 不做菜单栏项的显示开关（只显示图标、隐藏数字）。
- 不在菜单里放本月费用、最近的 session 等更多内容。
- 不做「点菜单栏图标直接开窗口」和右键菜单的区分。
- 不做「`⌘Q` 也只关窗口」。
- 不做多窗口。

### 验收标准

1. 启动后菜单栏出现图标和今天的费用，数字等于状态栏 Today 取整。
2. 点菜单栏项，菜单按上表的顺序出现，三行只读信息的数字和状态栏一致。
3. 按 `⌘W` 或点红色按钮：窗口消失，应用进程还在，菜单栏项还在，Dock 图标消失。
4. 窗口关着时，往数据目录追加新的用量，10 分钟内（或点 Scan Now 后）菜单栏的数字变化。
5. Open Sessions 重开窗口：Dock 图标回来，窗口在最前，过滤、排序、选中、搜索词和关掉前一样，键盘快捷键立即可用（不需要先点一下窗口）。
6. 窗口关着时在 Finder 里再次打开应用，效果同 Open Sessions。
7. 第一次关窗弹出说明对话框，选「好」后窗口关掉；之后再关不再弹；选「立即退出」则应用退出。
8. `⌘Q`、两处 Quit 都让进程退出。
9. 窗口关着时 Scan Now 可用，扫描期间它置灰、第三行显示 `Scanning…`，结束后显示新的时刻。
10. 从带 Sparkle 的正式包里 Check for Updates 仍可用；Install and Relaunch 能正常退出并重启。

## Tech Design

### 现状

- `crates/app/src/main.rs::main`：`gpui_kit::application().with_assets(AppAssets).run(..)`，里面 `cx.on_action(|_: &Quit, cx| cx.quit())`、`bind_keys`、`cx.set_menus`，然后 `cx.open_window` 创建唯一的窗口，根是 `Root`，里面是 `AppRoot`。`AppRoot` 拥有 `Store`（store 线程和 reader 线程的发送端）、`Searcher`、事件接收任务、usage-ledger 加的自动扫描定时器。
- GPUI（`gpui-pre-0.3.5/src/app.rs`）：`QuitMode::Default` 在 macOS 上等于 `Explicit`，关掉最后一个窗口不退出。窗口关掉后 `AppRoot` 被释放，`Store` 的发送端随之释放，store 线程的 `for request in requests` 结束。没有注册 `on_reopen`，所以点 Dock 图标没有反应。
- GPUI 提供的相关接口（已在源码里核对）：`Application::with_quit_mode` / `App::set_quit_mode`；`Application::on_reopen`（对应 `applicationShouldHandleReopen:hasVisibleWindows:`，只在没有可见窗口时回调）；`Window::on_window_should_close(cx, f)`，`f` 返回 false 时窗口不关（macOS 的 `windowShouldClose:`）；`Window::activate_window()`（`makeKeyAndOrderFront`）；`App::activate(true)`；`App::hide()`（隐藏整个应用）；`Window` 实现了 `raw_window_handle::HasWindowHandle`。测试用的 `VisualTestContext::simulate_close()` 会调用注册的 should-close 回调并返回它的结果。
- **GPUI 和 gpui-component 都没有菜单栏图标（`NSStatusItem`）的封装**（在 `gpui-pre`、`gpui-pre-macos`、`gpui-component`、`gpui-base`、`gpui-kit` 的源码里搜 `NSStatusItem` / `NSStatusBar` / `status_item` 都没有；`gpui-component` 的 `StatusBar` 是窗口底部的状态栏）。GPUI 启动时把 activation policy 设为 Regular（`gpui-pre-macos/src/platform.rs::did_finish_launching`），没有提供修改的接口。
- app 已依赖 `objc2 = "0.6.4"` 和 `objc2-foundation`；`updater.rs` 用 `msg_send!` 调 AppKit 和 Sparkle，把 `Retained<AnyObject>` 存在一个 GPUI `Global` 里。`objc2-app-kit 0.3.2` 已经在 `Cargo.lock` 里（GPUI 的依赖），app 直接依赖它不会引入第二个版本；它有 `NSStatusBar`、`NSStatusItem`、`NSStatusBarButton`、`NSMenu`、`NSMenuItem`、`NSImage`、`NSApplication`、`NSRunningApplication` 这些 feature。`objc2 0.6.4` 有 `define_class!`。
- 发布包的 `Info.plist` 由 `scripts/release/bundle-app.sh` 生成，没有 `LSUIElement`。

### 方案

**关窗改成隐藏窗口，不销毁。** `AppRoot` 注册 `on_window_should_close`，回调里把窗口隐藏（`orderOut:`）、把 activation policy 切到 Accessory，然后返回 false。窗口对象、`AppRoot`、`Store`、定时器、内存里的行和用量数据都原样留着，后台扫描不需要任何改动就继续跑；重开只是把同一个窗口再显示出来，视图状态自然和关掉前一样。

菜单栏项用 `objc2-app-kit` 自己建：新文件 `crates/app/src/resident.rs` 里创建 `NSStatusItem` 和它的 `NSMenu`，菜单项的 target 是一个用 `define_class!` 定义的小对象，它的 action 方法只做一件事：把一个命令发进 `async_channel`。GPUI 一侧有一个任务收命令并执行（显示窗口、扫描、退出）。`AppRoot` 在今天的数字或扫描状态变化时调用 `resident` 更新标题和三行只读信息。

关键决定：

- **隐藏而不是销毁并重建。** 备选是让窗口真的关掉，把 `Store`、事件分发、定时器、用量数据提到应用级的实体，重开时新建 `AppRoot` 并从 `state.json` 恢复。那要重排整个事件流（现在 `on_store_events` 按值消费事件），重开后搜索词和滚动位置也回不来。隐藏的代价是窗口关着时表格的行和上一次打开的对话还占着内存（几十 MB），可以接受。
- **隐藏单个窗口用 `orderOut:`，不用 `App::hide()`。** `App::hide()` 是隐藏整个应用，系统把它当作「被隐藏的应用」，Sparkle 的窗口也会跟着藏起来。`orderOut:` 只是让这个窗口离开屏幕。NSWindow 指针从 `HasWindowHandle` 取（AppKit 的句柄给的是 `NSView`，再取它的 `window`）。如果这条路在实现时走不通，退回 `App::hide()`，在 spec 里记下原因。
- **菜单项的回调经 channel 回到 GPUI，不在 AppKit 的回调里直接碰 GPUI。** 菜单的 action 在主线程的事件循环里触发，那时 GPUI 的 `App` 可能正被借用。发一个命令、由 GPUI 自己的任务在下一轮处理，不会重入。备选是借用 GPUI 的 Dock 菜单（`cx.set_dock_menu` 建出的 `NSMenu` 自带 action 分发）挂到 status item 上，但那个 `NSMenu` 取不到公开的句柄，要调代理的 `applicationDockMenu:` 去拿，依赖 GPUI 的内部实现。
- **图标先用系统符号**（`NSImage imageWithSystemSymbolName:`，template 图像，自动适配深浅色）。debug 二进制没有 bundle 资源，系统符号不需要资源文件。用哪个符号在实现时挑一个和「用量」相称的（如 `chart.bar.xaxis`），截图给用户看；以后要换成自己的图形再做一张 template 图放进 bundle。
- **activation policy 的切换是这一期最没把握的部分**（见「风险与待定」），所以实施的第一步是一个只验证这件事的试验，结论决定后面怎么写。用户已同意：做不稳就退回「Dock 图标一直在」。
- **测试里不建菜单栏项。** `resident::install` 只在 `main()` 里调用；`AppRoot` 拿到的是 `Option<Resident>`，测试里是 `None`，隐藏窗口的平台调用也经过它，所以无界面测试只验证状态和焦点。

### 数据变动

- `state.json`（`settings.rs::ViewState`）加 `resident_notice_shown: bool`，默认 false。旧文件没有这个字段时取默认值。
- `crates/app/Cargo.toml`：`cargo add objc2-app-kit@0.3.2 -p agent-session-manager --no-default-features --features NSStatusBar,NSStatusItem,NSStatusBarButton,NSMenu,NSMenuItem,NSImage,NSButton,NSControl,NSView,NSResponder,NSApplication,NSRunningApplication,NSWindow`（以实际编译需要为准）；`cargo add raw-window-handle -p agent-session-manager`，版本取 `Cargo.lock` 里 GPUI 用的那一个；`objc2-foundation` 按需要加 feature。
- 发布脚本和 `Info.plist` 不变（不设 `LSUIElement`：应用启动时有窗口，是普通应用）。

### 逻辑变动

**`resident.rs`（新）**

- `pub enum ResidentCommand { OpenSessions, OpenUsage, ScanNow, Quit }`。
- `pub struct Resident`：持有 `Retained<NSStatusItem>`、三行只读菜单项和 Scan Now 项的 `Retained<NSMenuItem>`、target 对象；实现 `Global`，像 `updater.rs::Updater` 那样存在 `App` 里。
- `pub fn install(cx: &mut App) -> Receiver<ResidentCommand>`：在主线程（`MainThreadMarker`）建 status item（`NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength)`），给按钮设图像和空标题，建菜单并挂上；返回命令的接收端。
- `Resident::update(&self, view: &ResidentView)`：设按钮标题和各菜单项的标题、Scan Now 的 enabled。`ResidentView { title: SharedString, today: SharedString, tokens: SharedString, scanned: SharedString, scanning: bool }` 由下面的纯函数生成。
- 纯函数：`title(today_usd: Option<f64>) -> String`（`None` 是空串；`203.69` → `$204`；`1203.5` → `$1,204`）；`menu_today(..)`、`menu_tokens(..)`、`menu_scanned(..)`。
- `hide_window(window: &Window)`：取 NSWindow，`orderOut:`。`set_dock_icon(visible: bool)`：`NSApplication::setActivationPolicy`（Regular / Accessory）。两者只在 `cfg(target_os = "macos")` 下有实现，其他情况是空函数。
- `define_class!` 定义 `AsmMenuTarget`（`NSObject` 的子类），四个方法 `openSessions:`、`openUsage:`、`scanNow:`、`quit:`，各自 `try_send` 一个命令；发送端存在它的 ivar 里。

**`main.rs`**

- `main()`：`with_quit_mode(QuitMode::Explicit)`（和 macOS 的默认相同，写明是为了不随 GPUI 的默认值变）。`resident::install(cx)` 在 `updater::install` 之后调用；起一个 `cx.spawn` 任务循环收 `ResidentCommand`，交给 `AppRoot::on_resident_command`（通过保存下来的 `WindowHandle<Root>` 和 `Entity<AppRoot>`）。`app.on_reopen(..)`：没有可见窗口时等同 `OpenSessions`。
- 新 action `CloseWindow`，绑 `cmd-w`（全局），菜单加 `Menu::new("File").items([MenuItem::action("Close Window", CloseWindow)])`，放在应用菜单后面。
- `AppRoot` 加 `window_hidden: bool`、`resident: Option<Resident 的句柄>`、`scan_failed_at`。
- `AppRoot::new`：`window.on_window_should_close(cx, ..)`，回调里调 `this.close_requested(window, cx)` 并返回 false。
- `close_requested(window, cx)`：第一次（`!saved.resident_notice_shown`）先用 `window.prompt(..)` 弹系统对话框，「好」之后继续、「立即退出」调 `cx.quit()`，并把标记写进 `state.json`。然后 `window_hidden = true`，`resident::hide_window(window)`，`resident::set_dock_icon(false)`。`CloseWindow` 的处理函数调同一个方法。
- `show_window(window, cx)`：`resident::set_dock_icon(true)`，`cx.activate(true)`，`window.activate_window()`，`window_hidden = false`，把焦点给当前视图的元素（会话表是表格；usage-view 之后按当前视图），最后 `auto_scan(cx)`。
- `on_resident_command`：`OpenSessions` → `show_window`（usage-view 合并后再加「切到会话表」）；`ScanNow` → `rescan(false, cx)`；`Quit` → `cx.quit()`。
- `on_store_events` 结尾：今天的合计、扫描状态有变化时调 `self.sync_resident(cx)`，它用纯函数算出 `ResidentView`，和上一次的不同才调 `Resident::update`。
- 窗口隐藏时 `StoreEvent::Failed` 不改变任何可见的东西，只记 `scan_failed_at` 供菜单第三行显示。`FavoriteChanged` 这类靠窗口通知的事件在隐藏时不会发生（它们由窗口里的操作引起）。
- 自动扫描的定时器（usage-ledger）不用改：它是 `AppRoot` 的任务，窗口隐藏后实体还在，照常醒来。

**`settings.rs`**：`resident_notice_shown`。

**文档**：`kb/docs/app-architecture.md` 的「键盘焦点」表加「窗口重新显示 → 当前视图的元素」，加一节说明窗口隐藏后各线程和定时器的状态；`kb/docs/gpui-kit-guide.md` 加菜单栏项和隐藏窗口的做法、`define_class!` 的用法；`AGENTS.md` 的验收流程里加「验收完用菜单栏的 Quit 或 `pkill` 结束应用，关窗口不再等于退出」，并把 `make dev` / `run` 的说明核对一遍（`run` 会先 pkill 旧实例，仍然成立）。

### 实施步骤

- [ ] 1. 试验（不进主线代码）：一个最小的 GPUI 窗口加 `resident.rs` 的雏形，验证四件事并把结论写进本 spec 的「风险与待定」：(a) `orderOut:` 之后 `activate_window()` 能把窗口带回来且能收到键盘输入；(b) Regular → Accessory → Regular 切换后，Dock 图标和应用菜单栏都恢复；(c) status item 的菜单项经 `define_class!` 的 target 能把命令送到 GPUI 的任务；(d) 菜单开着时更新菜单项标题是否即时生效（不生效也可以接受）。(b) 不稳定时停下来告诉用户，按「Dock 图标一直在」继续
- [ ] 2. `resident.rs` 的纯函数：先写测试，再实现
- [ ] 3. `settings.rs` 加字段和往返测试
- [ ] 4. `keyboard_tests.rs` 写关窗、重开、焦点的用例，再实现 `close_requested` / `show_window` / `CloseWindow`
- [ ] 5. `resident::install`、命令任务、`on_reopen`、`sync_resident`
- [ ] 6. 第一次关窗的对话框
- [ ] 7. `ASM_RESIDENT_AUTOPLAY` 和 `scripts/screenshot-resident.sh`（下节），实机验收；先读 `~/.claude/docs/DESKTOP_APP_TESTING.md`
- [ ] 8. `make release-local` 出一个本地包，验收标准 10 在包上做
- [ ] 9. 文档、commit

### 测试与验证

`resident.rs` 的单元测试（纯函数，对应验收 1、2、9）：

- `the_title_is_todays_cost_rounded_to_dollars`：`203.69` → `$204`，`0.4` → `$0`，`1203.5` → `$1,204`，`None` → 空串。
- `the_menu_lines_show_cost_tokens_and_the_last_scan`：`Today  $203.69`、`In 492.7M · Out 2.4M`、`Scanned 15:53`；扫描中是 `Scanning…`；失败是 `Scan failed 15:53`。

`settings.rs`：`a_saved_state_comes_back_unchanged` 加上新字段；旧文件缺字段时为 false（对应验收 7）。

`keyboard_tests.rs`（无界面窗口，`Resident` 为 `None`）：

- `closing_the_window_hides_it_and_keeps_the_app_running`（3、4）：先把 `resident_notice_shown` 写进测试用的 `state.json`；`simulate_close()` 返回 false，`window_hidden` 为真；之后往 fixture 追加一行并推进时钟 10 分钟，`AppRoot` 的行照常更新。
- `command_w_closes_like_the_red_button`（3）：`press("cmd-w")` 后 `window_hidden` 为真。
- `reopening_restores_the_view_and_gives_the_table_the_focus`（5）：设过滤、排序、选中一行、输入搜索词；关窗；调 `on_resident_command(OpenSessions)`；过滤、排序、选中、搜索框内容不变，表格有焦点，`press("n")` 能在命中之间跳。
- `the_first_close_asks_once`（7）：`resident_notice_shown` 为假时关窗，用 `cx.simulate_prompt_answer(..)` 回答「好」，窗口隐藏且标记写入；第二次关窗不再有对话框。回答「立即退出」的分支只断言调用了退出（测试平台能观察到的程度，观察不到就留给实机）。
- `scan_now_from_the_menu_scans_while_hidden`（9）：隐藏后 `on_resident_command(ScanNow)`，收到 `ScanDone`。

实机验收（`ASM_DB`、`ASM_STATE`、`ASM_CLAUDE_DIR` 指到 `tmp/2026-11-12-usage-stats/accept/`）。按项目规则不发系统级按键；菜单栏的弹出菜单无法程序化打开，用 cliclick 点击：

- `ASM_RESIDENT_AUTOPLAY=1`：启动扫描结束后依次执行并打日志标记：`autoplay-resident: shown` → 调 `close_requested`（标记已写入，不弹对话框）→ `autoplay-resident: hidden` → 等 3 秒 → `on_resident_command(OpenSessions)` → `autoplay-resident: reopened`。
- `scripts/screenshot-resident.sh`：每个标记后截两张图，一张是屏幕顶部的菜单栏条（`screencapture -R`，看图标和数字），一张是整个 Dock 区域（看图标在不在）；`hidden` 时用 `pgrep -fl agent-session-manager` 确认进程还在；再用 cliclick 点一次菜单栏项，截下拉菜单。对应验收 1、2、3、5。
- 验收 4：`hidden` 之后脚本往测试数据目录追加一条时间为现在的调用，点 Scan Now（cliclick），截菜单栏条，数字变化。
- 验收 6、8、10 手动做：隐藏后在 Finder 里双击 `make release-local` 产出的应用；三种退出方式各试一次并 `pgrep` 确认进程没了；包里 Check for Updates 能弹出 Sparkle 的窗口。
- 验收完用 Quit 或 `pkill -f agent-session-manager` 结束应用，`pgrep` 确认。

### 风险与待定

- **activation policy 来回切换（推测，未实测）。** 已知的情况：从 Accessory 切回 Regular 后，应用菜单栏有时不出现，要再激活一次应用，或者先让别的应用拿一下焦点。计划的顺序是「设 Regular → `activate(true)` → `activate_window()`」，不行再试延后一帧激活。步骤 1 实测；两种都不稳就按用户同意的退路做「Dock 图标一直在」（那时 `on_reopen` 负责点 Dock 图标重开窗口），并改 PRD 的对应条目。
- **隐藏的窗口在 GPUI 里的行为（推测）。** `orderOut:` 之后 GPUI 仍认为窗口存在；它是否继续为不可见的窗口安排绘制、`cx.notify()` 是否有额外开销，没有核实。后台扫描每次都会 `notify`，如果隐藏时仍在布局整张表，就在 `window_hidden` 时跳过 `on_store_events` 里只为显示服务的重算（表格的 `after_rows_changed` 仍要做，数据要保持最新）。
- **`windowShouldClose:` 只管用户关窗。** `NSApp terminate:`（`⌘Q`、Sparkle 重启、关机）不经过它，所以这些路径不受影响；步骤 8 在正式包上确认。
- **Sparkle 在 Accessory 状态下弹窗**：窗口关着时它的更新提示可能出现在别的应用后面。不处理，登记为已知的表现。
- **菜单开着时数字不刷新**：菜单打开期间扫描结束，标题更新可能要等菜单关掉再开才看到。可以接受。
- **debug 二进制的 Dock 图标**是通用图标，切换 policy 的效果以正式包为准。
- 第一次关窗的对话框用 `window.prompt`（GPUI 的系统对话框）；它在测试平台上由 `simulate_prompt_answer` 回答，接口名以本地源码为准。
