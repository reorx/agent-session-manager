---
created: 2026-09-28
updated: 2026-11-16
tags:
  - architecture
  - threads
  - store
  - search
  - persistence
  - agent-session-manager
---

# 运行时架构

写给要改数据流的人：索引扫描、用量与自动扫描、搜索、详情数据的读取、视图状态的保存。回答三个问题：什么工作跑在哪条线程上，事件按什么顺序到达界面，改动时哪些规则必须保持以及原因。

本文只描述现状。界面组件的做法在 [gpui-kit 开发指南](gpui-kit-guide.md)，表结构与存储选型在 [数据层设计](data-layer-design.md)，各里程碑当时的实测与取舍在 `kb/notes/`。

## 线程

| 线程 | 持有 | 工作 |
|---|---|---|
| 主线程 | 全部行、过滤排序后的顺序、选中、视图状态、内存里的用量（`UsageData`）、菜单栏项（`resident::Resident`，GPUI 全局） | 渲染；过滤与排序；应用后台事件；算今天的费用；定时发起自动扫描；更新菜单栏项、执行它的菜单命令 |
| `asm-store` | 索引的读写连接（`Db` 自带用量账本的连接）、模型目录 | 读索引、扫描并写索引（同时记账）、读账本、读取模型窗口、收藏与目录迁移 |
| `asm-search` | 索引的只读连接 | 文本搜索查全文索引；正则搜索扫 transcript 文件 |
| `asm-reader` | 无（不碰索引） | 给详情面板直接读一个 session 的记录文件（或 History session 在 `history.jsonl` 里的 prompt），转换成回合卡；写导出的 TXT |
| rayon 线程池 | 无 | 扫描时并行解析文件，正则搜索时并行扫文件 |

界面向后台线程发请求，后台线程用事件回应。两个方向都是无界队列，发送方不等待接收方。主线程给每条后台线程配一个接收任务。

这样划分的原因：

- **一个连接只在一条线程上用，写索引的只有 store 线程。** 写者唯一就不需要锁，也不会出现两个写者互相等待。解析是并行的，写库在 store 线程的单个事务里完成。
- **搜索有自己的线程和连接。** store 线程一次只做一件事，一次冷扫描要 3–4 秒，搜索排在它后面的话输入时会卡住。索引是 WAL 模式，读和写可以同时进行。
- **读记录文件也有自己的线程。** 理由同上：冷启动或 Rebuild 期间选中一行，对话不能等扫描结束。它只读文件、不开连接，store 线程在写库也不受影响。线程由 `Store::spawn` 一起启动，事件走 store 的同一条通道。
- **过滤和排序留在主线程。** 5488 行过滤加排序 0–1ms，10k 行排序 7ms，送到后台再传回来更慢，还多出一套同步问题。后台只做 SQLite 查询和文件扫描。

## 启动与扫描

store 线程启动后先自己走完一遍固定流程，然后才开始处理请求：

```text
读取模型目录并打开索引 → 读出全部行 → 读出账本的全部用量 → 增量扫描 → 重读模型目录 → 记录或窗口有改动则再读一次全部行
```

它发给界面的事件：

| 事件 | 内容 | 界面的处理 |
|---|---|---|
| `Loaded` | 索引里的全部行 | 整表替换 |
| `Usage { full: true }` | 账本里全部 session 的小时汇总，紧跟第一个 `Loaded` | 整份替换 `UsageData`，算今天的费用 |
| `UsageUnavailable` | 这次运行没有账本（更新版本写的账本，或读不了） | 状态栏 `Today —` |
| `Progress` | 已处理与总文件数；安静的扫描不发 | 状态栏进度 |
| `Session` | 扫描刚写入的一行（新增或有变化）；安静的扫描等事务提交后才一起发 | 按 session id 新增或替换 |
| `Usage { full: false }` | 这次扫描写过的 session 在账本里的小时汇总，在 `ScanDone` 之前；没有就不发 | 按 session id 替换，算今天的费用 |
| `ScanDone` | 扫描报告、消息数、索引大小、是不是安静的扫描（`quiet`） | 状态栏摘要（安静的扫描什么都没变时不改）；结束扫描计时 |
| `ScanSkipped` | 安静的扫描等不到索引或账本的锁，回滚了；在它之前这次扫描什么都没发 | 只结束扫描计时，界面没有反应 |
| `Conversation` | 读取线程读好的回合卡（`ConversationView`）或出错信息，带 `seq` 和读取 / 转换耗时 | 见下文"详情的读取" |
| `FavoriteChanged` | 收藏 / 取消收藏完成 | 通知提示；行沿 Session / Loaded 更新 |
| `FavoritesDirChanged` | 新目录、迁移与跳过数 | 更新路径并通知 |
| `RebuildingRoots` | 配置的根目录变化 | 通知自动重建原因 |
| `Failed` | 出错信息 | 状态栏显示 |

不同情形下的顺序：

```text
热启动，文件没有变化
  Loaded(全部行) → Usage(全部) → Progress → ScanDone

热启动，有文件变化
  Loaded(全部行) → Usage(全部) → Progress / Session … → Usage(写过的) → ScanDone → Loaded(全部行)

冷启动（索引为空）
  Loaded(0 行) → Usage(全部) → Progress / Session … → Usage(写过的) → ScanDone → Loaded(全部行)

Rebuild
  Loaded(0 行) → Progress / Session … → Usage(写过的) → ScanDone → Loaded(全部行)
```

- 热启动在第一个 `Loaded` 到达时表格就可用，之后的扫描只是核对。本机 5488 个 session，从界面创建到行上屏 187–254ms，随后的增量扫描 0.07–0.66s
- 冷启动靠 `Session` 事件填表，首批行约 1.0s 出现，全部完成 3.6–4.0s，期间表格可以滚动和操作
- Index 菜单的 Rebuild 先清空索引，之后与冷启动相同，只是不重读账本（账本不随索引清空）；Scan for Changes 从"增量扫描"那一步开始，没有开头的 `Loaded`
- session 没有变化的扫描不发任何 `Session`；模型目录窗口也没变时，不触发最后的 `Loaded`，所以热启动时表格不会闪，搜索也不会重跑
- 启动时读取 Claude 数据目录下的模型缓存，每次扫描发完 `ScanDone` 后重读。只比较归一化后的窗口表，获取时间或无关字段变化不刷新；窗口变化（含文件损坏或删除导致回退）即使没有 session 改动也补发 `Loaded`。不监听缓存文件
- `Loaded { sessions, catalog, elapsed }` 把 `Arc<ModelCatalog>` 与整表一起交给主线程，主线程先替换目录再构造行；流式 `Session` 使用最近一次 `Loaded` 的目录。Ctx 比例、排序与 Metadata 都从同一份窗口计算，SQLite 不存窗口，目录变化无需重建索引

### subagent 的记录文件

扫描也看 subagent 的记录文件（`<session>/subagents/*.jsonl`），和 session 的记录文件走同一套增量记账，但只读其中每次 API 调用的 token 用量，算进它所属 session 的费用。进度条只数 session 的记录文件。

- **先写 subagent 的用量，再写 session 行。** 这样每个 `Session` 事件里的费用已经包含 subagent，一个 session 在一次扫描里只发一次
- **只有 subagent 变了的 session 也会发 `Session`。** 它自己的记录文件没变，扫描在最后重算它的费用再发出；这种扫描不触发最后的 `Loaded`（流出的行已经是准确的）
- **subagent 文件消失**：session 自己的记录文件还在，说明是单独删掉的，它的用量一起删掉、费用下降；记录文件也不在了，说明整个 session 被 Claude Code 清理，用量保留，和 session 自己的用量一样（钱已经花了）
- 冷启动多读约 200MB，耗时从 3.45–3.53s 变成 3.62–3.77s

### 事件成批应用

主线程的接收任务每次醒来先取一条事件，再把队列里积压的全部取走，整批只重算一次表格、只重绘一次。冷启动会流进几千条 `Session`，逐条重算会卡住界面；成批处理后只重算几十次。

### 扫描之后为什么要整表重读

扫描发现记录或模型窗口有改动时，`ScanDone` 之后会再发一次 `Loaded`，用索引里的最终结果与当前窗口替换整张表。记录改动需要重读的原因有三个：

1. **流出的行是写入那一刻的快照，可能不完整。** 已知的情况是项目路径滞后：一个没有工作目录记录的空 session 先于同项目的其他 session 写入时，它的项目路径还是未知的，界面只能显示编码后的目录名。同项目后面的 session 写入后索引里的项目路径才确定，但已经流出去的那一行不会再发一次
2. **它是搜索重跑的信号。** 扫描的全部写入在一个事务里，搜索线程的连接在事务提交前看不到新内容。重读发生在提交之后，界面收到它就知道可以重新搜索了
3. **代价很小。** 5488 行读出来约 25ms

流式事件只会新增或替换行，从不删除行。重读是表格里的行与索引重新完全一致的唯一时机。

## 用量与今天的费用

用量账本（core 的 `ledger.rs`，索引同目录的 `usage-ledger.sqlite`）在 `Db` 里接上：store 线程写索引时同时记账，界面不碰账本。账本只增不删，所以界面的 `UsageData` 也只替换、不删除：

- **只有启动时整份读一次**（`Usage { full: true }`）。之后每次扫描只发它写过的 session（`Usage { full: false }`，按 id 替换）。Rebuild 清空的是索引不是账本，这次扫描读不到的 session（记录文件已删、又不在 history.jsonl 里）不会出现在事件里，界面保留原来的用量（`store.rs` 的 `a_rebuild_does_not_drop_usage_the_ui_already_has`）
- **今天的费用在主线程算**：`roll_day` → `refresh_today` 对 `UsageData` 求 `[今天 4:00, 下一个 UTC 整点)` 的合计。上界取下一个整点而不是现在，因为账本按 UTC 整点存小时桶，取现在会漏掉当前这个小时。状态栏的 `Today …` 在这里格式化成字段，render 只取
- **用量日**（`AppRoot::usage_day`，4:00 起）是会话表时间预设、日期选择器快捷项和今天的唯一依据。它不在 render 里算：`roll_day` 在收到用量或扫描结束、窗口激活、定时器醒来时检查，换了日就重新应用时间过滤（`set_range`），再重算今天。所以跨过 4:00 后，最迟到下一次检查（不超过 10 分钟）才换日
- 账本不可用（`UsageUnavailable`）时状态栏是 `Today —`，其余照常。还没收到账本时状态栏不显示这一段

## Usage 视图

主窗口有两个视图（`AppRoot::view`，`model.rs` 的 `MainView`）：会话表和 Usage。切换只有一个入口 `AppRoot::set_view`（`⌃1` / `⌃2`、View 菜单、工具栏左端的 `Sessions | Usage`、状态栏的 `Today $…`、菜单栏菜单的 Open Sessions / Open Usage 都走它）。

- **Usage 屏幕上的一切来自一份快照**（`usage_view.rs` 的 `UsageSnapshot`），由纯函数 `UsageSnapshot::build(UsageData, 账本是否可用, 查询, 现在, 用量日, 时区)` 生成，render 只取其中的字符串。快照只在 Usage 显示时重算（`refresh_usage`）：切到 Usage、`roll_day`（收到用量、扫描结束、窗口激活、定时器醒来都会经过它）、过滤变化（`apply_filter`，只在 Project / Kind / Model 变了时）、范围 / 指标 / 点选某天变化。会话表显示时调用它直接返回；内容和上一份相同就不 `notify`，所以没有变化的扫描界面不动。在真实账本副本上重算一次 1.5–4.5ms
- **过滤与会话表共用**：Project、Kind（CLI / SDK / BG）、Model 就是会话表的那几个，在 Usage 里改了，回到会话表照样生效。Model 存的是去掉日期和 `[1m]` 的名字（`pricing::normalize`，账本就这样记模型），会话行的主模型（`SessionRow::primary_model`）也存成这样，两边才对得上；旧版本存下的带日期的值在比较时同样去掉。隐藏的项目、空 session、收藏、History、会话表的时间范围不影响 Usage（花掉的钱都要算）。Usage 的时间范围是自己的一套（`UsageState::range` / `custom`），和会话表的 Updated 互不影响
- **Usage 显示时不搜索**：扫描后的整表重读在 Usage 下只记下 `search_stale = Refresh`，在 Usage 里改过滤只记下 `Rerun`（取较重的那个）；回到会话表时 `set_view` 按记下的跑 `refresh_search` 或 `run_search`。原因是 Usage 每分钟扫一次，正则搜索一次可能要几秒，看不见的结果没必要跟着重跑
- **只属于会话表的动作在 Usage 下直接返回**：Toggle Detail、收藏、Preview 1–4、隐藏项目、显示空 session、命中 / 回合跳转、正则 / 大小写开关。`⌘F` 先切回会话表再聚焦搜索框；Escape 在 Usage 下是 `leave_day`（退出点进去的那一天）
- **点选某一天**（`drill_into_day`）只在多天的图上、那天有调用时生效，`range` 变成 `Day`，记下来的 `back` 是点之前的范围；Escape 或小标签的 ✕ 调 `leave_day` 回去。保存视图状态时 `Day` 存成 `back`

## 自动扫描

应用开着时自己做增量扫描（`AppRoot::auto_scan`），触发有四个：窗口变为激活（`observe_window_activation`），窗口从菜单栏重新显示（`show_window`，日志 `auto-scan: window shown`），切到 Usage 视图（`set_view`，日志 `auto-scan: usage shown`），和一个循环任务（`_auto_scan`，`spawn_auto_scan`）：每次睡之前读 `auto_scan_interval`，Usage 显示且窗口没有隐藏时是 `USAGE_SCAN_IDLE`（60 秒），否则 `AUTO_SCAN_IDLE`（600 秒）。切换视图、窗口隐藏和重新显示时换一个新任务（旧任务 drop 即取消），等待从头算，所以切回会话表后不会再在一分钟内扫一次。菜单的 Scan for Changes / Rebuild Index 和菜单栏菜单的 Scan Now 照旧是手动扫描（`rescan`）。

| 规则 | 原因 |
|---|---|
| 上一次扫描（任何来源，包括启动时那次和菜单发起的）还没结束，或结束不到 `AUTO_SCAN_MIN_GAP`（30 秒），就跳过 | 切窗口很频繁；扫描排在 store 线程上，叠在一起会让收藏等请求排队 |
| 发的是 `Scan { quiet: true }`，不把状态栏改成 `loading index…` | 没有变化时界面不应有任何反应 |
| 安静的扫描什么都没变时，`ScanDone` 不改状态栏中间的摘要 | 同上；菜单发起的扫描照旧写「index up to date」 |
| 安静的扫描不发 `Progress`，写过的行先攒在 store 线程，事务提交后再发 | 等锁的那几秒状态栏不变成 `indexing n/m`；等不到锁时整个事务回滚，回滚掉的行不能进表格。账本也会被锁：索引写完、提交前写账本时等不到，同样回滚 |
| 安静的扫描遇到索引或账本被锁（`is_busy`），store 线程只发 `ScanSkipped`，不发 `Failed` | `asm` 正好在扫描时不报错，等下一次 |
| 有变化时和手动扫描一样整表重读、搜索重跑 | 规则只有一套；重搜是静默的，见「搜索」 |

计时用 `cx.background_executor().now()`，不用 `Instant::now()`：测试里 `advance_clock` 能推进它，30 秒和 10 分钟的用例不用真等（`keyboard_tests.rs` 的 `scans_triggered_within_thirty_seconds_run_once`、`ten_minutes_later_…`）。`scan_in_flight` 在 `AppRoot::new` 里就是 true（store 线程打开索引后马上扫描），由 `ScanDone` / `ScanSkipped` / `Failed` 清掉；所以启动时窗口第一次激活不会多扫一次。

日志：发起时打 `auto-scan: <触发>`，结束打 `scan (automatic): <报告>`，今天的费用变了打 `today: …`，换日打 `usage day: <旧> → <新>`。

## 窗口隐藏之后

关窗不是关窗：红色按钮（`on_window_should_close` 的回调返回 false）、`⌘W` 和 File > Close Window（`CloseWindow` action）都走 `AppRoot::close_requested`，它把窗口 `orderOut:` 掉、把 activation policy 切到 Accessory（没有 Dock 图标、不在 `⌘Tab`、菜单栏交还给别的应用），记下 `window_hidden`。窗口对象、`AppRoot`、`Store`、搜索线程、所有任务都原样留着，所以：

| 部分 | 窗口隐藏时 |
|---|---|
| store / 读取 / 搜索线程 | 照常运行，请求和事件照常进出 |
| 自动扫描的定时器 | 每 600 秒醒一次（是 `AppRoot` 的任务，实体还在）；隐藏前停在 Usage 也是 600 秒，重新显示后回到每分钟 |
| 事件的应用 | 照常：行、用量、今天的费用都更新到 `AppRoot` 里 |
| 渲染 | 不发生：GPUI 在窗口被遮挡时停掉 display link，`cx.notify()` 只标脏，重开时一次重绘（usage-p2-probe 实测隐藏期间 `render` 调用 0 次）。所以 `on_store_events` 不需要为隐藏的窗口跳过任何计算 |
| 菜单栏项 | `sync_resident` 在扫描开始 / 结束、收到用量、换日时把标题和三行信息推过去，内容没变就不推 |
| 出错 | `Failed` 只改状态栏（看不见）和菜单第三行的计数；不弹通知。通知只由窗口里的操作引起，窗口隐藏时不会发生。store 线程已经不在时（索引打不开），扫描当场算失败，见「出错时」 |

菜单栏菜单的命令从 AppKit 进来：菜单项的 target（`resident::MenuTarget`，`define_class!` 定义的 NSObject）只把一个 `ResidentCommand` 送进 `async_channel`，`AppRoot::listen_to_menu` 起的任务收到后调 `on_resident_command`。AppKit 的回调里不碰 GPUI 的 `App`（那时它可能正被借用）。Finder / Spotlight 再次打开应用时 AppKit 调 `applicationShouldHandleReopen:`（只在没有可见窗口时），`main()` 里的 `on_reopen` 往同一条通道送 `OpenSessions`。

重开（`show_window`）的顺序固定：先切回 Regular，再 `cx.activate(true)`（`activateIgnoringOtherApps:`），再 `window.activate_window()`，然后焦点给表格、`roll_day`、`auto_scan("window shown")`。用户点了 Open Sessions、或在 Finder 里再次打开应用时，在这之前还有一步：AppKit 的回调里当场调 `resident::bring_forward`（切 Regular，再 `NSApp.activate()`），之后才把命令送进通道。原因：macOS 14 起激活是协作式的，`NSApp.activate()` 紧跟用户自己的操作时才生效（试验：真实点击后 1/1，定时器触发 0/4）；`activateIgnoringOtherApps:` 不需要点击，前台是终端、Finder 时有效，前台是游戏时 3 次都被拒绝（usage-p2-accept）。两条都走：点击和 Finder 靠前者，autoplay 这类没有点击的路径靠后者。实测（usage-p2-resident-r1，Finder 在前台）：点击 Open Sessions、程序化选 Open Sessions、`open` 再次打开 bundle，三种都 `active=true`，菜单栏换回本应用的菜单，Dock 图标回来。Finder 再次打开时 LaunchServices 先把应用（还是 Accessory）激活了，回调里才切 Regular（日志 `active before: true`），菜单栏照样回来。`activateIgnoringOtherApps:` 已被 Apple 标为弃用，哪天失效了，autoplay 拿不到前台，用户的路径不受影响。

第一次关窗（`state.json` 的 `resident_notice_shown` 为假）先弹 `window.prompt`（窗口上的 sheet），答案经一个任务回来：「好」隐藏窗口，「立即退出」退出；两种都把标记写进 `state.json`。对话框开着时再次关窗什么都不做。

退出只有 `cx.quit()` 一条路（`⌘Q` 的 `Quit` action、菜单栏菜单的 Quit、「立即退出」）；`NSApp terminate:`（Sparkle 的 Install and Relaunch、注销、关机）不经过 `windowShouldClose:`，不受影响。`main()` 写明 `QuitMode::Explicit`，最后一个窗口关掉也不退出（窗口其实从不关）。

### 扫描期间请求要排队

store 线程是单循环，扫描进行中到达的请求（收藏、取消收藏、换收藏目录）要等扫描结束。详情面板不受影响：它的对话走读取线程（下一节），冷启动或 Rebuild 期间选中一行，对话照常出现。

## 详情的读取

`Store::read_conversation(session_id, 记录文件路径, 项目目录, matcher)` 把请求交给 `asm-reader` 线程，返回这次请求的 `seq`。线程读文件（core 的 `read_conversation`，给了 matcher 就顺便标出命中），再在同一条线程上转换成回合卡（app 的 `conversation::build`），发出 `Conversation` 事件。

读取线程的队列里是 `ReadRequest`，三种：

| 请求 | 入口 | 做什么 | 回应 |
|---|---|---|---|
| `Conversation` | `Store::read_conversation` | 读记录文件 | `Conversation` |
| `History` | `Store::read_history(session_id, project_key, matcher)` | 读 `history.jsonl` 里这个 History session 的 prompt（core `history_prompts`，分组和表格一致），每个 prompt 一张卡（`conversation_from_history`） | `Conversation`，和记录文件的同一种事件 |
| `Export` | `Store::export(记录文件, 目标路径)` | core `read_transcript` 得到和 `transcript-reader` 逐字相同的 TXT，写到目标路径 | `Exported { dest, result }`：成功是跳过的坏行数，失败是说明。不走 `Failed`，那个事件会把状态栏改成索引出错 |

- **新请求取代旧请求，做法照搬搜索，只对 `Conversation` / `History` 生效。** 它们共用一个 `seq`，发请求时加一。线程每次醒来先把队列里积压的请求取完，其中的导出按到达顺序全部执行，`Conversation` / `History` 只读最后一个；开始读之前和读完之后各看一次最新的 `seq`，过期就不发。所以快速换行时只有最后选中的那个 session 被读，界面收到的 `Conversation` 事件序号只增不减；界面仍要按 `seq` 和当前选中的 session 核对再用。导出没有 `seq`，换行再快也不会被丢掉（`store.rs` 的 `an_export_is_not_superseded_by_a_conversation_request`）
- **History session 走同一条路。** 选中它（`SessionRow.transcript_path` 为 `None`）时 `AppRoot::read_conversation` 发的是 `read_history`，返回同一种 `Conversation` 事件，取代、核对、搜索重读都一样。`ConversationView.history` 为 true，每张卡是 `prompt_only`：只有卡头（`TURN n`、时间、prompt），行表里一张卡就是一行 `Row::Head`，没有状态灯、刻度和卡尾；顶栏写 `N prompts from history.jsonl`，顶栏下面一块虚线框说明记录文件已被清理。搜索模式不读 history（和 Python 脚本一致），所以搜索时表格不会列出 History session，它的卡片在界面上碰不到命中标记；读取线程照样会标（`store.rs` 的测试）
- **转换也在读取线程上做。** 卡片上的全部字符串（时间、用时、统计行、结果摘要、折叠位置）都在转换时算好，主线程只拿 `Arc<ConversationView>` 生成行表（`conversation::rows`）和绘制。记录文件读完就在读取线程上释放，内存里只留截断后的文本。release 下 70MB 的 `5f6eea99` 读取约 18ms、转换 0.2ms；465 次工具调用的 `4e5802b6` 读取 21ms、转换 1.1ms
- **`ConversationView.pattern` 记着读取时用的 `Matcher::pattern()`**（带大小写标记）。搜索词变了要重读才能重新标记（详情里没有全文的缓存）；pattern 相同就不必重读
- **读不了文件不会带走线程。** 文件被删、没有权限时，事件里是 `Err(说明)`，下一个请求照常处理

界面这一侧（`main.rs`、`toolbar.rs`）：

| 时机 | 做什么 |
|---|---|
| 选中换了 session | 详情清空成骨架（`DetailState::begin_loading`，上一个 session 的内容不留在屏幕上，Metadata 的文件列表也收起），带当前搜索的 `Matcher` 发请求（有记录文件的 `read_conversation`，History session 的 `read_history`），记下返回的 `seq` 和这次请求的 pattern |
| 收到 `Conversation` | `seq` 等于最新一次请求、`session_id` 是当前选中的才用。屏幕上已经有这个 session 的对话（为新搜索重读）就保留展开状态，否则全部折叠；都停在第一个命中位置，没有搜索时回到顶部 |
| 搜索重跑（`run_search`；扫描后的 `refresh_search` 不改查询，不读） | 新 `Matcher::pattern()` 和最近一次请求的 pattern 不同才重读；过滤变化、扫描后整表重读引起的重搜 pattern 不变，不读文件（`keyboard_tests.rs` 有测试）。重读期间旧内容留在屏幕上 |
| 读取失败 | 对话区显示事件里的说明 |

每次换 session，第一帧画出对话时 stdout 打一行 `detail: <短 id> <大小> read <ms> build <ms> first frame <ms>`，起点是 `select_row`，终点是这一帧的 paint（列表后面的一个 `canvas` 在 paint 阶段打这行）。debug 下 70MB 的 `5f6eea99` 约 read 21ms、build 0.7ms、first frame 50–70ms。

## 操作入口

复制 resume 命令、复制 session ID、在 Finder 中显示记录文件、打开项目目录、导出 TXT、收藏，都在 `actions.rs`。详情标题行的按钮、表格行的右键菜单、`⌘C` / `⇧⌘C` 三个入口派发同一组 action，处理函数只写一遍：

- **action 带 `id: Option<String>`。** 右键菜单派发 `Some(那一行的 id)`，作用于右键点中的行（不一定是选中的行，右键不改选中）；快捷键绑定的是 `None`，和标题行按钮一样作用于选中的行
- **能不能做由纯函数 `actions::available(row, 原记录在不在, 项目目录在不在)` 决定**，每项是 `Ok` 或带英文原因的 `Err`。不能做时标题行按钮置灰、悬停提示写原因，菜单项置灰，快捷键弹通知说明原因、什么也不复制
- **可用性要碰文件系统（`is_dir`、收藏的原记录 `is_file`），只在需要时由 `actions::check` 做**：选中一行时（`select_row`，结果存在 `DetailState::actions`，标题行和 Metadata 的 Resume 一行读它）、打开右键菜单时、执行 action 时各做一次，每次两三个 stat。不放进 `SessionRow::from_summary`：那是整表加载的路径，几千行逐个 stat 不值得。`SessionTableDelegate` 创建时带着 `ClaudeDir`，菜单才能查收藏的原记录
- **恢复命令**：项目目录在时 `cd '<项目目录>' && claude --resume <id>`（路径总是单引号，里面的单引号写成 `'\''`），目录不在时 `claude --resume <id>`（Claude Code 2.1.223 起按 id 在所有项目里找）；只剩收藏副本时不提供。依据见 `kb/sessions/2026-10-27-asm-m4b-resume.md`
- **导出**：保存对话框的默认目录是这次运行里上一次导出的目录（`AppRoot::last_export_dir`，不持久化），第一次是"下载"；默认文件名 `<项目名>-<id 前 8 位>.txt`。选好路径后交给读取线程写，`Exported` 到了再弹通知

## 收藏与目录切换

收藏、取消收藏和更换目录都在 store 线程执行，与扫描串行。界面发出 `Favorite`、`Unfavorite` 或 `SetFavoritesDir`，文件操作完成后扫描更新行，再发送完成通知。取消最后一份副本前，界面等待原生确认框；文件选择器和确认框关闭后把焦点还给当前视图。

扫描先发现原目录和收藏目录，再按 session id 去重，收藏始终优先。路径离开本次发现集合后会移除其增量状态；同 id 有另一份胜出时从头重读，避免旧消息或费用重复。所有主文件和 subagent 状态最终只指向胜出的路径。

索引 meta 保存两个根目录。CLI 使用不同根目录扫描同一个索引会拒绝操作，提示指定其他 `--db` 或显式重建；桌面启动遇到根目录变化自动重建。正常迁移先移动文件，再保存共享配置并更新 meta，最后扫描；失败回滚已移动文件。

收藏路径配置保存在索引旁的 `config.json`，只有「仅看收藏」属于 `state.json` 视图状态。

## 搜索

搜索的对象由界面决定：通过当前过滤条件的 session，从新到旧。搜索线程只在这个范围里找，最多返回最新的 200 个有命中的 session。所以搜索结果始终是过滤结果的子集，不需要在两边各实现一遍过滤条件。

文本模式一次返回全部结果；正则模式每扫完一批文件就返回一批，结果边扫边出。

### 新搜索取代旧搜索

每次搜索都有一个递增的序号。发起新搜索时序号加一，正在进行的正则扫描每处理一个文件检查一次，发现自己过期就停下；已经发出的旧事件到了界面也会因为序号不符被丢弃。没有排队，也没有"等上一次搜完"。

新搜索的第一条事件到达之前，表格保留上一次的结果。每敲一个字就清空表格的话，输入过程中表格会一直闪。

### 什么时候重新搜索

| 触发 | 说明 |
|---|---|
| 搜索词变化 | 停止输入 150ms 后；按 Enter 立即执行 |
| 搜索选项变化 | 模式、大小写、Scope、Labels |
| 过滤条件变化 | 搜索对象变了 |
| 扫描后的整表重读 | 索引内容或模型窗口变了；仅窗口变化也照常重搜 |

程序化地改写搜索框不会产生"内容变化"事件，改写之后要自己触发搜索。

流式到达的 `Session` 不触发重搜，原因见上文"扫描之后为什么要整表重读"第 2 点。

### 扫描后的重搜是静默的

前三种是用户发起的新搜索：第一批结果到达时清空表格换上新结果并滚到顶部，状态栏先显示 `searching…`。扫描后的整表重读不是：自动扫描每 10 分钟就可能来一次，用户正往下翻搜索结果时不能被拉回顶部。所以重读走 `refresh_search`：

- 同样的查询再搜一次，命中先攒在 `AppRoot::search_refresh` 里；表格留着旧结果，状态栏留着旧摘要
- 搜索结束（`Done` 或 `Failed`）时用 `replace_hits` 一次换上全部命中，不滚动，再按 `relocate_selection(false)` 重新定位选中：还在结果里就留在原 session 上，不在了照旧退回第一行（这时会滚到第一行）
- 只在屏幕上的就是搜索框里这个查询的结果时才这样做；搜索框已经改了、或者新搜索的第一批结果还没到，就当新搜索跑。用户发起的任何搜索都会清掉 `search_refresh`，被取代的静默重搜的事件按序号丢弃

`keyboard_tests.rs` 的 `an_automatic_scan_does_not_scroll_search_results_back_to_the_top` 和 `a_selected_session_gone_from_refreshed_results_hands_the_selection_to_the_first_row` 锁住这两点。

## 选中

选中跟着 session 走，不跟着行号走。数据、过滤条件、搜索结果或排序（包括点表头）变化后，按下面的顺序决定选中哪一行：

1. 原来选中的 session 还在表格里：移到它的新位置
2. 不在了，表格非空：选第一行
3. 表格为空：清空选中，详情面板清空

两个例外，都是为了避免表格乱跳：

- **扫描正在流式填表、且当前没有选中时，不自动选第一行。** 设置选中行会顺带滚动到该行，而扫描期间第一行一直在变。等扫描结束再选
- **启动时要恢复上次的选中，但那一行还没出现时，先等。** 行出现就选中它；扫描结束仍然没有，才按第 2 条选第一行

**扫描引起的刷新不把表格滚回选中的行。** 自动扫描每 10 分钟就可能来一次，选中的 session 因为别的 session 排到前面而换了行号时，`set_selected_row` 会顺带把它滚进视口，用户正往下翻的表格就被拉回去。所以 `on_store_events` 走 `relocate_selection(false)`：选中的还是同一个 session、只是换了行时，照样 `set_selected_row`（高亮跟着 session 走），但在同一个 update 里把它留在列表滚动句柄上的延迟滚动请求（`vertical_scroll_handle.0.deferred_scroll_to_item`）拿回去，下一帧就不会滚。新选中的行（恢复上次的选中、退回第一行）照常滚进视口；过滤、排序引起的重新定位也照常滚动。搜索模式下扫描还会引起重搜，那条路径见「扫描后的重搜是静默的」。`keyboard_tests.rs` 的 `an_automatic_scan_does_not_scroll_the_table_back_to_the_selected_row` 锁住这一点（不这样做时滚动位置会从 −86px 回到 0）。

点表头排序是表格组件自己做的，它重排行却不改选中的行号。所以 `AppRoot` 观察表格时记下上一次的排序，一变就按上面的规则重新定位选中；记录要在重新定位之前更新，因为重新定位会再通知一次表格。

表格的 `SelectRow` 事件是通知，不是命令：表格先改好自己的选中行再发事件，事件要等当前 update 结束才处理。一次 update 里选了两次，就会排两个事件，只有和表格当前选中行一致的那个才处理，旧的丢掉。如果旧事件也照做，选中会被拉回旧行、再发一个事件，两个事件来回触发，主线程空转（autoplay 第二步曾因此卡死）。

详情面板只在选中的 session 换了之后才重新读对话。同一个 session 的行被扫描更新、点表头重排时，详情不重新加载（`keyboard_tests.rs` 的 `matches_header_sort_keeps_the_selected_session_and_detail_hit`）。对话返回时如果选中已经换成别的 session，或者之后又发过请求，结果丢弃。

## 键盘焦点

快捷键和菜单项都是发给"有焦点的元素"的动作，沿着它的祖先一路向上找处理者。窗口里没有任何元素有焦点时，动作只到得了窗口的根，到不了应用自己的处理者：除了退出（它注册在应用级），快捷键和菜单项都不起作用。

所以焦点任何时候都应该在应用的某个元素上：

| 时刻 | 焦点 |
|---|---|
| 启动 | 当前视图：表格，或上次停在 Usage 时 Usage 视图（`focus_view`） |
| `ctrl-1` / `ctrl-2`、View 菜单的 Sessions / Usage、工具栏的切换、状态栏的 `Today $…` | 切到的视图：表格或 Usage 视图（`usage_view.focus`，key context `Usage`） |
| `cmd-f` | 搜索框（在 Usage 里先切回会话表） |
| 搜索框里按 Enter 或 Escape | 表格 |
| 在日期区间选择器里选完区间（或点了快捷项） | 表格；Usage 工具栏的那个选择器还给 Usage 视图 |
| 导出 TXT 的保存对话框、更换收藏目录的文件选择器、取消最后一份收藏的确认框关闭 | 当前视图（`focus_view`）：更换收藏目录在 Usage 里也能用，那时表格不在屏幕上，焦点给它就等于没有焦点（审阅 R1） |
| 窗口重新显示，不管关窗前焦点在哪 | Open Sessions：表格；Open Usage：Usage 视图；Finder / Spotlight 再次打开（`ResidentCommand::Reopen`）：关窗前的那个视图 |

新增会拿走焦点的界面（弹出层、对话框）时，关闭后要把焦点还给当前视图（`focus_view`）或搜索框；只在会话表里出现的界面才可以直接给表格。

`ctrl-1` / `ctrl-2` 不绑上下文，搜索框里也能切（`⌘1`–`⌘4` 已是 Preview 模式）。单键快捷键都绑在表格的上下文（`"DataTable"`）上，搜索框里打字不受影响：`n` / `shift-n` 在命中之间跳，`[` / `]` 在卡片之间跳。`[` / `]` 只在详情面板显示、且停在 Conversation tab 时起作用（`AppRoot::step_turn`）：把下一张卡（回合或 AUTO，不含 compact 分隔线）的卡头滚到顶端，`[` 先回到当前卡的开头；到第一张或最后一张不动，不改变展开状态（`DetailState::step_turn`，`keyboard_tests.rs` 的 `brackets_*` 两个测试）。

## 视图状态的保存

| 保存 | 不保存 |
|---|---|
| Preview 模式 | 搜索词 |
| 排序的列与方向 | 按 Matches 列的排序（离开搜索后这一列就不存在了） |
| 项目筛选、时间范围（含自定义区间的起止日）、Kind 筛选 | 详情面板当前的 tab |
| 隐藏的项目、是否显示没有 prompt 的 session | 窗口大小与位置、分栏宽度 |
| 详情面板是否可见 | 滚动位置 |
| 选中的 session | |
| 搜索选项（模式、大小写、Scope、Labels） | |
| 第一次关窗的说明是否已经显示过（`resident_notice_shown`） | |
| 停在哪个视图（`view`） | Usage 里点进去的那一天（存成点之前的范围） |
| Usage 的范围（`usage_range`）、自定义区间的起止日（`usage_custom_range`）、图的指标（`usage_metric`） | Usage 的滚动位置 |

规则：

- **凡是可能改变上述状态的操作之后都尝试保存，内容与上次保存的相同就不写盘。** 表格的任何变动都会触发一次尝试，包括与视图状态无关的变动，所以比较是必须的
- **读取时尽量宽容。** 文件不存在或内容损坏用默认值；缺少的字段取默认值，不认识的字段忽略。新版本加字段不需要迁移，旧文件照常可用。新加的字段必须有合理的默认值，例如 Kind 筛选的默认是全选
- **选中还在等待恢复时保存，写进去的是等待恢复的那个 id。** 否则启动后行还没出现就改了一次过滤条件，上次的选中就丢了
- 文件是索引同目录下的 `state.json`，环境变量 `ASM_STATE` 可以改到别处。验收时用一次性的文件，保证从默认视图开始

## 出错时

- 后台线程的顶层把所有错误转成 `Failed` 事件，界面在状态栏显示。单个请求失败不会带走线程
- 索引打不开时 store 线程先关掉请求通道，再发出 `Failed`，然后结束，表格保持为空。之后的扫描请求（定时、窗口激活、Scan Now、菜单的 Scan / Rebuild）发不出去，`Store::request` 返回 false，`AppRoot::request_scan` 当场按扫描失败处理：不留在扫描中，菜单第三行是 `Scan failed HH:MM`，状态栏保留打不开的原因（之前没有出错信息时显示 `cannot scan: the index is not open, restart the app`）。先关通道再报错，是为了让 app 收到 `Failed` 之后发的请求一定被拒，不会丢在通道里等一个永远不来的结束事件。索引文件损坏、版本不符或 schema 与本版本建出的不一致都不算打不开：打开时会清空重建，之后按冷启动处理
- 搜索失败时搜索线程丢弃自己的连接，下次搜索重新打开。它的连接在第一次文本搜索时才建立，那时 store 线程已经把索引建好了

## 相关文件

- `crates/app/src/store.rs` — store 线程、读取线程、请求与事件类型；文件头注释有启动顺序，测试断言了冷启动、热启动、Rebuild 的事件序列，用量事件（首次整份、之后只发写过的、Rebuild 不丢）、安静的扫描遇到锁时跳过，以及读取在扫描期间照常返回、新请求取代旧请求、History 的 prompt、导出与导出不被取代
- `crates/app/src/actions.rs` — 操作的可用性（`available` / `check`）、恢复命令、导出文件名、各 action 与处理函数
- `crates/app/src/conversation.rs` — 回合卡的转换（`build`）与行表（`rows`、`card_rows`、`row_of`、`fold`）；`real_transcript_timings` 是量真实文件读取与转换耗时的 ignored 测试
- `crates/app/src/detail.rs` — 详情的状态（`DetailState`：加载状态、行表、展开 / 折叠、命中位置与停靠）
- `crates/app/src/turn_cards.rs` — 回合卡的绘制（`list` 的每一行），刻度的悬停与点击
- `crates/app/src/search.rs` — 搜索线程、取代机制、结果上限
- `crates/app/src/main.rs` — 两个接收任务、事件的成批应用、选中规则（`after_rows_changed`）、对话的请求与采用（`open_conversation`、`show_conversation`）、保存（`persist`）
- `crates/app/src/toolbar.rs` — 过滤条件的应用、搜索的防抖与重跑；视图切换和 Usage 的工具栏（范围、`Custom…`、三个下拉、`as of`、立即扫描）
- `crates/app/src/usage_view.rs` — Usage 视图：`UsageState`、`UsageSnapshot::build`（纯函数，测试断言它的字符串）、`refresh_usage`、范围 / 指标 / 点选某天、各区块的绘制
- `crates/app/src/table.rs` — 全部行与过滤排序后的顺序、搜索对象的生成
- `crates/app/src/settings.rs` — 视图状态的读写与容错测试
- `crates/app/src/resident.rs` — 菜单栏项与菜单（`install`、`Resident`、`MenuTarget`）、隐藏窗口与 activation policy、菜单文字的纯函数（标题取整、三行信息、连续失败的判定）
- `crates/app/src/keyboard_tests.rs` — 启动焦点与搜索的键盘路径（`cmd-f`、Enter、Escape、`n`、`cmd-g`）、`cmd-c` / `cmd-shift-c` 复制（用测试平台的剪贴板）的行为测试；状态栏的 Today、自动扫描（定时、窗口激活、30 秒节流、不改摘要、不滚回选中行）、跨过 4:00 换日；关窗即隐藏（红色按钮、`cmd-w`）、隐藏时定时扫描与 Scan Now、重开后视图不变且焦点在表格、第一次关窗的说明只弹一次；Usage 视图的切换与焦点、只属于会话表的快捷键在 Usage 下无效、点选某天与 Escape、共用的过滤、每分钟扫描与隐藏后回到十分钟、Usage 下推迟的搜索、重启后恢复视图
- `crates/core/src/ledger.rs`、`crates/core/src/usage.rs` — 账本与 4:00 分日、`UsageData` 的合计；表结构与迁移规则见 [数据层设计](data-layer-design.md)
- `crates/core/src/index/indexer.rs` — 流式扫描与单事务写入
- `kb/notes/2026-09-21-m2-real-data.md`、`kb/notes/2026-09-21-m3-search.md` — 上述数字的来源与当时的取舍
