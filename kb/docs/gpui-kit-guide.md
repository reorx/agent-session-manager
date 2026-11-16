---
created: 2026-09-28
updated: 2026-11-16
tags:
  - gpui
  - gpui-kit
  - ui
  - guide
  - agent-session-manager
---

# gpui-kit 开发指南

写给要改 `crates/app` 界面代码的人：gpui-kit 0.6.1 在本项目里实际是怎么用的，哪些行为和文档说的不一样，哪些限制绕不开。

这里只写在本项目代码和本地源码里核实过的内容。GPUI 的通用心智模型（Entity、Context、notify、虚拟列表）、组件目录和选型理由在 [GPUI 框架调研](gpui-framework-research.md)；两份文档说法不一致时以本文为准，本文没覆盖的以本地源码为准。

## 依赖与查 API

应用只声明 `gpui-kit = "=0.6.1"` 一个 UI 依赖。它把 GPUI 和组件库一起重导出，再单独声明 `gpui` 或 `gpui-pre` 会让两份 GPUI 同时进入依赖图，类型互不相认。直接调 AppKit 的地方（Sparkle、菜单栏项）用 `objc2`、`objc2-foundation`、`objc2-app-kit` 和 `raw-window-handle`，版本取 `Cargo.lock` 里 GPUI 已经在用的那一个（`cargo add …@<版本>`），不会引入第二份。

| 路径 | 内容 |
|---|---|
| `gpui_kit::*` | GPUI 本身 |
| `gpui_kit::component::*` | 带样式的组件库（表格、按钮、菜单、输入框…） |
| `gpui_kit::base` | 无样式的行为层 |
| `gpui_kit::actions!` | 定义 Action 的宏；GPUI 原版宏在 facade 下解析不到 `gpui::Action` |

在线文档和 docs.rs 对不上锁定的版本，查签名直接读本地源码：

```sh
ls ~/.cargo/registry/src/index.crates.io-*/ | grep -E 'gpui-(component|base)-0.6.1|gpui-pre-0.3.5'
grep -n "pub fn " ~/.cargo/registry/src/index.crates.io-*/gpui-component-0.6.1/src/table/state.rs
```

registry 里同时有 `gpui-pre-0.3.4` 和 `0.3.5`，`Cargo.lock` 解析到的是 0.3.5。

debug 构建能用，靠的是根 `Cargo.toml` 的 `[profile.dev.package]` 给重依赖设了 `opt-level = 3`（GPUI、组件库、文本排版，以及 SQLite、serde、core crate）。这一段必须写在根 `Cargo.toml`。不设的话 debug 版冷索引 7 秒以上还跑不完。

## 应用骨架

启动顺序是固定的：`gpui_kit::init(cx)` 必须先于任何组件，`style::install_theme(cx)` 必须在它之后（否则配色被默认主题覆盖）。窗口的第一层必须是 `Root`，顶层 render 末尾要挂上 sheet、dialog、notification 三个层，否则这些弹出内容不会显示。

```text
Root
└── AppRoot
    ├── 工具栏        「标签 值 ▾」下拉 · 搜索框 · 搜索模式开关
    ├── h_resizable   左：DataTable    右：详情面板（可隐藏）
    ├── StatusBar     行数 · 索引状态 · 消息数与 DB 大小
    └── sheet / dialog / notification 层
```

快捷键和菜单项都是 Action：`gpui_kit::actions!` 定义，`cx.bind_keys` 绑定，`AppRoot` 的根元素上用 `.on_action(cx.listener(..))` 接收。

## 数据怎么进界面

界面不直接碰 SQLite，也不解析 JSONL。索引和搜索各有一条后台线程，界面向它们发请求、收事件；只有 SQLite 查询和文件扫描放后台，过滤和排序在主线程做。线程划分、事件顺序、搜索何时重跑、选中与保存的规则在 [运行时架构](app-architecture.md)，改这些之前先读。

对写界面代码的人来说，要记住的是：core 类型到界面行只有一个转换点，所有格式化（时间、大小、预览截断）在那里一次做完，渲染时只取结果。

## 表格

表格是 `DataTable`，由一个 delegate 供数。delegate 持有全部行和一个索引向量，索引向量是过滤加排序后的结果，表格的第 n 行就是索引向量的第 n 项。过滤或排序只重建索引，不移动行数据。

**渲染单元格时不做任何计算。** 每帧都会对可见单元格调用一次，任何格式化、查找、分配都会直接反映在滚动帧率上。单元格只取预格式化好的字符串。

排序同值时按 updated 降序，保证顺序稳定；没有排序时的默认顺序也是 updated 降序。

### 行高与表头

| 限制 | 影响 |
|---|---|
| 行高固定，单元格 `overflow_hidden` | Preview 列用 36px 两行布局：标题一行，灰色预览一行，各自截断 |
| 表头行高与数据行高是同一个值 | 表头是 36px，设计稿是 24px，无法单独设 |
| 排序图标的渲染函数是私有的 | 所有可排序列都显示 ⇅，设计稿只在当前排序列显示箭头 |
| 表格尾部有一个空列，外加纵向滚动条 | 最后一列右侧留了约 16px |

需要真正的可变行高只能放弃 `DataTable`，改用虚拟列表自建。

### 列宽

表格横向也做了虚拟化：面板比列总宽窄时出现横向滚动，右边的列被裁掉。`Column` 没有弹性宽度，所以 Preview 列的宽度由应用自己算：每次 render 读左侧面板的当前宽度，减去其余列的固定宽度，变化超过 0.5px 才写回并刷新表格。首帧面板尺寸还是空的，用窗口宽度减详情面板宽度兜底。

列可以从表头右键菜单隐藏（2026-11-12）：delegate 留着全部列定义（`all_columns`）和隐藏集合，显示的 `columns` 由二者重建，搜索时的 Matches 插在 Preview 后面。所以 Preview / Matches 的位置要按 key 查，不能写死下标；其余列的宽度对显示的列求和，不再是常量。Preview 的宽度同时写进两份定义，重建时才不会丢。

### 排序状态

表头箭头的状态存在列定义里，而表格每次 `refresh` 都会向 delegate 重新要列定义。Preview 列宽同步会在窗口尺寸变化时触发 `refresh`，所以列定义必须始终反映当前排序，否则一拖窗口箭头就跳回默认列。

做法是以 delegate 自己记录的排序为准，每次排序后同步到列定义。表格自己的排序入口是私有的，程序化排序（恢复上次状态、autoplay）走 delegate 的 `set_sort`，之后调用 `refresh` 让箭头跟上。

### 选中行

`TableState::set_selected_row` 会顺带滚动到该行，没有只选中不滚动的入口。所以不能在行还在变动时反复设置选中，否则表格跟着跳。

要只改选中、不滚动（扫描刷新后选中的 session 换了行号），可以利用它的滚动是延迟执行的：`set_selected_row` 只往公开的 `vertical_scroll_handle`（`UniformListScrollHandle`）里放一个 `deferred_scroll_to_item`，下一帧 prepaint 时才滚。调用前把原来的请求 `take()` 出来，调用后放回去，新请求就被丢掉了（`main.rs` 的 `select_row_scrolling`）。要放回原来的请求而不是直接置空：同一帧里可能还有别处刚要求过滚动（如 `scroll_to_row`）。目标行本来就在视口里时 uniform list 不会滚，所以只有行被挤出视口时才看得出差别。选中按 session id 保持以及由此而来的两个例外，规则在 [运行时架构](app-architecture.md) 的"选中"一节。

列选中默认是开的：点表头会选中整列，整列的高亮会盖掉行选中。本项目的表头点击只用于排序，所以建表时关掉了（`col_selectable(false)`）。

## 主题与密度

设计稿的全部色值集中在 `style.rs`，按名字取用。别处不写 hex，这样界面才能和设计稿逐项比对。

- **全局密度**靠把主题字号设成 12px。`Root` 每帧用主题字号设置 rem，组件内部以 rem 计的尺寸随之按比例缩小，正好落到设计稿的密度。需要精确尺寸的地方显式写 `px()`
- **改主题色要做三步**：改 `colors`；用 `colors` 重建 `tokens`（表格和状态栏读的是 `tokens`，只改 `colors` 它们看不到）；调用 `Theme::sync_base` 同步到行为层（滚动条等）
- **选中色必须半透明**：表格把选中行画成一个绝对定位的层，盖在单元格**上面**。不透明的颜色会把整行内容遮住。当前是强调色 16% 不透明度，叠在白底上约等于设计稿的 `#dde8fa`；选中边框设为透明以去掉 1px 描边
- **通知（`window.push_notification`）在右下角**，状态栏上方 12px：`theme.notification.placement = Anchor::BottomRight`、`margins.bottom`（`install_theme`）。默认的右上角会盖住工具栏右端、Usage 的 Sessions 卡 `↗` 和详情面板标题行的按钮约 5 秒，正好是下一下要点的地方（usage-p4-accept 问题 4：一次真点击落在了通知上）。通知整块 `.occlude()`，盖住的地方点不下去

## 下拉菜单与按钮

工具栏的下拉是 `Button` 加 `.dropdown_menu(..)`，没有用 `Select`。

- 菜单内容的构造函数在每次打开时重新执行，所以直接捕获当前值来决定哪一项打勾即可，不需要维护菜单状态
- 菜单项用 `PopupMenuItem::element(..)` 放任意内容，`.checked(bool)` 打勾，回调里通过弱引用更新 `AppRoot`
- 靠右的下拉用 `dropdown_menu_with_anchor(Anchor::TopRight, ..)`，否则菜单会伸出窗口
- 按钮外观：`ButtonCustomVariant` 定底色和 hover 色，边框、高度、圆角直接链在 `Button` 上。实例样式最后生效，能覆盖变体
- 搜索框旁的开关（`.*`、`Aa`）也是自定义外观的 `Button`，开和关各用一套颜色，没有用 `Switch` 或 `ToggleGroup`。悬停说明用 `.tooltip(..)`
- `trigger`、`toggle`、`tool_button` 都挂了 `debug_selector("toolbar-<id>")`（`toolbar-project`、`toolbar-preview`、`toolbar-regex`、`toolbar-usage-custom`…），工具栏本身是 `toolbar`、会话表的搜索框是 `toolbar-search`。`debug_selector` 不改布局，测试据此查位置

### 窄窗口里的工具栏

两个视图的工具栏都 `.flex_wrap()`：`h_auto().min_h(TOOLBAR_H).py(4)`，一行放不下就换行，单行时的位置和不换行时一样（会话表默认 1440px 的控件位置改前改后逐个对过，`tmp/2026-11-12-usage-stats/usage-p4-drill/r1/toolbar-1440-*.txt`）。

- 靠右的一组（会话表的 Preview、Usage 的 `as of` + 刷新）用 `ml_auto()`，不用 `div().flex_1()` 占位：换行后它停在所在那一行的最右边；`flex_1` 的占位宽度是 0，换行时留在上一行末尾，右边那组会跑到下一行的最左边
- 换行时元素按 flex-basis 断行，不先压缩：固定 `w(220)` 的搜索框会让工具栏在还挤得下时就换行。会话表的搜索框写成 `flex_basis(160).flex_grow(1.).max_w(220)`，宽裕时长到 220，挤的时候先缩到 160 再换行
- 分隔线不能单独落在一行末尾或开头。哪一行断在哪取决于标签长度（项目名），写死阈值不可靠；Usage 工具栏用 `on_children_prepainted` 拿到这一帧各子元素的 bounds，分隔线和前后两个控件在同一行才画（`UsageState::separator_shown`，变了才 `notify`）。不画时用 `.invisible()`，照样占位：要是拿掉它，宽度少 11px，可能又放得下一行，下一帧再出现，来回闪

### 右键菜单与带参数的 action

表格行的右键菜单由 `TableDelegate::context_menu(row_ix, menu, ..)` 构造，每次打开时调用，`row_ix` 是右键点中的行（`TableState::right_clicked_row`），不是选中的行。菜单项派发的 action 从表格的焦点向上冒泡，到达 `AppRoot` 的 `on_action`。

- **菜单右侧的快捷键提示是按 action 相等去查绑定的**（`popup_menu.rs::render_key_binding`）。本项目的 action 带 `id: Option<String>`：菜单项派发 `Some(id)`，快捷键绑定的是 `None`，两者不相等，所以普通的 `menu(..)` 项不会显示 `⌘C`。要显示就自己画：`PopupMenuItem::element(|_, _| 左标签 + 右提示).action(Box::new(..)).disabled(..)`。核实过（M4b spec 风险第 3 条）：element 项支持 `disabled`，点击时照常派发 action 并关闭菜单，不需要退回 `menu_with_disabled`
- **element 项的外层 `h_flex` 没有 `min_w_0`**，内容里有一段很长的不换行文字（resume 命令预览）时，它会按整段文字撑宽，`truncate()` 不起作用，右侧提示被推到菜单外。给自绘内容一个固定宽度：菜单宽度减去菜单边距 4px 和项内边距 8px（两侧共 24px），见 `table.rs` 的 `MENU_ITEM_W`
- 普通项置灰用 `menu_with_disabled(label, action, disabled)`。菜单项没有悬停提示，置灰的原因只能写在项里（本项目把它写在 Copy Resume Command 的预览行）
- **表头的右键菜单**（显示哪些列）组件不提供，在 `render_th` 返回的元素上自己挂 `ContextMenuExt::context_menu`，元素要有稳定的 id（按列 key）。表格的行菜单挂在整个表格上，右键表头时也会构造，它读 `right_clicked_row`：为 `None` 时菜单为空，空菜单不弹出；但上一次右键过的行可能还留着，所以表头格子在右键按下时先 `set_right_clicked_row(None)`。右键不会触发表头的排序（`on_click` 只认左键）。实测见 `kb/plans/2026-11-12-columns-and-cache-breaks.md`

### 按钮的图标与置灰

- **补图标**：`gpui_kit::assets::Assets` 只带组件库用到的 101 个图标。要别的 Lucide 图标，用 `gpui_kit::assets::icon_assets!(ExtraIcons, [SquareTerminal, ..])` 生成只含这几个的 `AssetSource`，再写一个自己的 `AssetSource`：先查 `ExtraIcons`，查不到回落到 `Assets`，`list` 合并去重（`main.rs` 的 `AppAssets`，照 gpui-kit-assets README 的写法）。变体名是文件名的驼峰形式（`file-search.svg` → `FileSearch`），`gpui_kit::assets::IconName` 里能直接用
- **图标尺寸**：`Button::icon(..)` 按按钮尺寸缩放图标，以 rem 计；本项目 rem 是 12px，小号按钮的图标只有 10px 左右。要画稿的 14px，就把 `Icon::new(..).with_size(px(14.))` 作为 child 放进按钮
- **置灰的按钮照样显示悬停提示。** gpui-base 的 `Button` 置灰时只拦 mouse down 和 click，悬停不拦，`tooltip(..)` 照常出现。所以标题行置灰的按钮把提示换成原因
- **置灰的外观**：自定义变体（`ButtonCustomVariant`）置灰时背景是底色的 15%、边框是底色的 15%（白底就看不见了）。实例上链的 `.border_color(..)` 在置灰状态下也会覆盖回来，再加 `.opacity(0.4)` 就是画稿的样子
- 带快捷键的按钮用 `tooltip_with_action(文字, &action, Some("DataTable"))`，提示里会画出键帽
- **可点的行里再放一个按钮**（Usage「按项目」行末尾的 `↗`）：行的 `on_click` 和按钮的 `on_click` 都会收到这次点击，按钮的处理函数里先 `cx.stop_propagation()`，行的就不会再触发。`keyboard_tests.rs` 的 `a_click_on_a_project_s_arrow_jumps_and_does_not_also_click_its_row` 用测试窗口的鼠标点击锁住这一点（去掉这一行它就失败）

## 搜索框与键盘

搜索框是 `Input`，状态在 `InputState` 里，两者分开：状态在构造视图时创建并订阅事件，render 时用 `Input::new(&state)` 画出来。

- **程序化写入不发事件。** `set_value` 在写入期间关掉了事件发送，订阅者收不到 `Change`。代码里改了搜索框的内容（清空、autoplay 填词）之后，要自己调用后续逻辑
- **Escape 会冒泡。** 没有开 `clean_on_escape` 时，输入框处理完自己的事（收起多光标、取消输入法组字）就把 Escape 交给上层。本项目靠这一点在 `AppRoot` 上接住它：清空搜索并把焦点交还表格。开了 `clean_on_escape` 则输入框自己清空并吞掉按键，上层收不到
- **Enter 是事件，不是 Action。** `PressEnter` 从订阅里来；本项目用它立即搜索并把焦点交给表格
- **防抖靠替换任务。** 每次 `Change` 都新建一个"等一会儿再搜索"的任务并存进同一个字段，旧任务随之被 drop 而取消

**窗口刚打开时没有焦点，此时任何快捷键和菜单项都到不了 `AppRoot`。** 动作从有焦点的元素沿祖先向上分发，没有焦点就只到窗口的根。`AppRoot` 创建时把焦点给了表格；规则在 [运行时架构](app-architecture.md) 的"键盘焦点"一节。

**单键快捷键要限定上下文。** 组件各有自己的 key context（表格是 `"DataTable"`，输入框是 `"Input"`），绑定时把它作为第三个参数，快捷键就只在该组件有焦点时生效。`n` / `shift-n`、`[` / `]` 绑在表格的上下文上，所以在搜索框里打字不会被吞掉。带修饰键的快捷键不受打字影响，可以绑成全局。

## 命中高亮

高亮用 GPUI 核心的 `StyledText::with_highlights`，没有用组件库的 `Label::highlights`。后者接收的是查询字符串并自己做大小写不敏感的匹配，而本项目的命中位置由 core 算出（多词、正则、大小写开关），界面只负责按给定的区间上色。

- 区间是 **UTF-8 字节偏移**，必须落在字符边界上，debug 构建会断言
- 区间要按起点升序、互不重叠。源码按顺序累加各段长度，不排序也不合并
- 高亮样式（背景色、前景色）和其他色值一样集中在 `style.rs`
- 放在会截断的单元格里时，截断发生在右侧，排在后面的高亮可能被裁掉。表格的片段因此让首个命中尽量靠前

## 行内代码和代码块

prompt、过程文字和回复按段显示（`conversation::segments`）：围栏代码块一段，其余是正文，正文里记着行内代码的区间。区间都是原文的字节区间，命中不用换算，渲染时按段切开（`turn_cards.rs` 的 `marked`）。工具的输入输出、thinking、compact 摘要不分段。

**行内代码和命中画在同一段 `StyledText` 里，用 `with_highlights` 加 `with_font_family_overrides`，不用 `with_runs`。** M4b spec 设想的是自己拼 `Vec<TextRun>`，但 `TextRun` 要给出完整的字体和颜色，而这些是从父元素继承的（prompt 是 medium 字重，AUTO 卡是灰字，回复是正文色），自己拼就要把每处的样式抄一遍。`with_highlights` 在布局时按继承来的样式算 run，只覆盖给出的那几项；`with_font_family_overrides` 换字体族，两者可以一起用（`gpui-pre-0.3.5/src/elements/text.rs`）。

- **字体覆盖只对整个落在覆盖区间里的 run 生效**（`apply_font_family_overrides`）。run 的边界来自高亮区间，所以高亮必须在行内代码的每个边界上都切开，否则跨边界的那一段命中不会换成等宽字。`conversation::text_runs` 把行内代码、反引号、命中的边界合在一起切，每一段标上是不是代码、是不是反引号、是不是命中（测试含中文，切点都在字符边界上）
- **命中盖在行内代码上面**：命中那段用命中的高亮（黄 / 橙底），字体仍是等宽；不是命中的代码段用灰底（`style::inline_code_bg`）
- **高亮的颜色是叠加的，不是替换**：`TextStyle::highlight` 里 `color = 原色.blend(新色)`，所以给透明色等于什么都没改。要把字藏起来用 `fade_out: Some(1.)`（把 alpha 乘到 0）。反引号就是这样藏的：它们留在文字里（区间不变），不画出来，正好当灰底左右的留白
- 字号不能按 run 换（`TextRun` 没有字号），行内代码和正文同一个字号，只换字体和底色；画稿里的 11px 和 3px 内边距做不到

**代码块**是一个单独的框（`turn_cards.rs::code_block`）：`.whitespace_nowrap()` 让文字不按宽度折行（`TextLayout` 只在 `WhiteSpace::Normal` 时给折行宽度，换行符照常分行），`.overflow_hidden()` 加 `.min_w_0()` 把超宽的部分裁掉；框里的命中照常用 `with_highlights`。

**分段后命中的停靠标记**（`reveal_marker`）要两份布局：命中所在那段的 `TextLayout`（算命中的位置），和第一段的（算整段文字的顶端）。只看命中那段的顶端，命中排在一个长代码块后面时就会以为它离顶端很近而不滚动。

## 不定高的长列表：`list` 与 `ListState`

详情面板的对话用 GPUI 核心的 `list(state, render_item)`，不用组件库的 `v_virtual_list`：后者要预先给出每一项的高度，自动换行的文本算不出来。`list` 只布局看得见的项（加上 `ListState::new` 的 overdraw 那一段），量出的高度缓存在 `ListState` 里，视口外的项按估算高度算。源码在 `gpui-pre-0.3.5/src/elements/list.rs`。

**一项是卡片的一段，不是一张卡。** 界面持有一张行表 `Vec<Row>`（`conversation::Row`：卡头、一步、卡尾、compact 分隔线），列表的第 n 项就是行表的第 n 行。卡片的外观由各行拼成：卡头画上边框和上圆角，卡尾画下边框和下圆角，中间的行只画左右边框。展开 200 步的回合是 200 个小项，每帧只布局看得见的那几个，命中也能定位到具体某一步。

| 要做的事 | 做法 |
|---|---|
| 整张表换掉（换 session、换搜索） | 先改行表，再 `list.reset(rows.len())`：所有项变成未测量，滚动回到开头 |
| 展开 / 收起一个回合 | 只替换变了的那几行：比较这张卡新旧两份行表，去掉相同的头尾，`list.splice(from..to, 新行数)`，行表做同样的 `splice`。插在视口上方的行会把滚动位置一起后移，视口里的项不动（`detail.rs` 的测试锁住了这一点）。整张卡都 `splice` 的话，视口顶端那一项会被重置到开头，看着会跳 |
| 一行的内容变了高度（展开一步、Show all） | `list.remeasure_items(ix..ix + 1)`。不要对它 `splice`：理由同上 |
| 滚到某一行 | `list.scroll_to(ListOffset { item_ix, offset_in_item: px(0.) })`；回到开头用 `ListOffset::default()`，滚到最后用 `scroll_to_end()` |
| 滚动条 | 外层容器 `.vertical_scrollbar(&list_state)`，`ListState` 实现了 `ScrollbarHandle` |

`ListState` 内部是 `Rc<RefCell<..>>`，克隆就是同一份状态；它不需要窗口就能构造、`splice`、`scroll_to`，`logical_scroll_top()` 在纯逻辑测试里可以断言。

**回调里拿 `AppRoot`。** `list` 的回调签名是 `FnMut(usize, &mut Window, &mut App) -> AnyElement`，拿到的不是 `Context<AppRoot>`。渲染时先 `let view = cx.entity();`，回调里 `view.update(cx, |this, cx| this.render_row(ix, window, cx))`，这样行里照常能用 `cx.listener(..)`。回调在 `AppRoot::render` 返回之后的布局阶段才执行，那时实体没有被借用，`update` 不会冲突。可见的项每帧都重新 render，所以悬停这类只改外观、不改高度的状态，`cx.notify()` 就够了。

**行里的元素 id 要按内容取，不按行号。** 每一项是单独布局的根，不同项里同名的 id（例如 `"tick"`）会共用交互状态。用 `ElementId::NamedInteger(name, (card << 32) | step)`。

**跳到文字中间的某个位置。** `scroll_to` 只能把一行的顶端对齐到视口顶端，而一行可能是一段很长的 prompt 或工具输出，命中在几屏之下。`list` 支持子元素在 prepaint 里调用 `window.request_autoscroll(bounds)`：它会在同一帧里调整滚动位置，让这块区域进入视口（区域在下方时底边对齐视口底边），然后重新布局。本项目的做法（`turn_cards.rs::reveal_marker`）：停靠时置一个一次性的标记，含当前命中的那段文字后面跟一个零尺寸的 `canvas`，它的 prepaint 读同一段 `StyledText` 的 `TextLayout::position_for_index(命中起点)`，命中离文字顶端超过 240px 才请求一块从命中上方三行开始、约一个列表高的区域，请求完清掉标记。`position_for_index` 在文字没有布局或 prepaint 之前会 panic，放在后面的兄弟元素的 prepaint 里调用正好避开：兄弟元素按顺序 prepaint，轮到 `canvas` 时前面的文字已经布局完了。只量不画的项（overdraw）不会调用 prepaint，标记留给真正画出来的那一帧。

**首帧时间。** 要知道"内容画出来"的时刻，在列表后面放一个 `canvas`，用它的 paint 回调打点：paint 按元素顺序进行，轮到它时列表已经画完。`window.on_next_frame` 不合适：回调在下一帧**开始**、绘制之前执行。

定高、条数有上限的内容（Metadata）仍用普通滚动容器：`.overflow_y_scrollbar()`，或 `.track_scroll(&ScrollHandle)` 加 `.vertical_scrollbar(&handle)`。

## 元素函数的返回类型

edition 2024 下，`fn f(cx: &App) -> impl IntoElement` 的返回值会捕获 `cx` 的生命周期，同一个表达式里连续调用两次就借用冲突。带 `cx` 参数的辅助函数返回具体类型（`Div`、`AnyElement`、`StatusBar`），或者写 `-> impl IntoElement + use<>`。

## 异步任务

- `Task` 被 drop 就取消。`cx.spawn(..)` 的结果要么 `.detach()`，要么存进字段；存字段顺带得到取消能力
- 任务里更新视图的调用返回 `Result`，视图已销毁时失败。用 `.ok()` 或据此退出循环，不要 unwrap
- 闭包里持有视图用弱引用，避免循环引用

## 菜单栏项与隐藏窗口

GPUI 和 gpui-component 都没有菜单栏项（`NSStatusItem`）的封装，`gpui-component` 的 `StatusBar` 是窗口底部的状态栏。`resident.rs` 直接用 `objc2-app-kit`（0.3.2，GPUI 已经依赖的那个版本，只开用到的 feature）建，做法和坑都来自 usage-p2-probe 试验（`tmp/2026-11-12-usage-stats/usage-p2-probe/report.md`）：

| 要点 | 做法 |
|---|---|
| 菜单项的回调 | target 是 `define_class!` 定义的 NSObject 子类（类名在进程内唯一，`AsmMenuTarget`），方法里只 `try_send` 一个命令进 `async_channel`；GPUI 一侧的任务收命令、`update_in` 执行。AppKit 回调里不碰 GPUI 的 `App`。例外是 Open Sessions：回调里当场调 `bring_forward`（切 Regular、`NSApp.activate()`），趁点击还是用户最近的操作，见下一行。菜单先发 `menuDidClose:` 再发 action，命令执行时菜单已经关了 |
| 持有 | `NSMenuItem.target` 和 `NSMenu.delegate` 都是弱引用，`NSStatusItem` 释放了菜单栏项就消失：`Resident`（GPUI `Global`，像 `updater.rs` 的 `Updater`）持有 item、菜单、要改标题的菜单项和 target |
| 置灰 | `menu.setAutoenablesItems(false)`，否则 AppKit 按 target 能否响应自动启用菜单项，`setEnabled` 不生效 |
| 更新 | 在 GPUI 主线程的代码里直接 `setTitle` / `setEnabled`，菜单开着时也即时生效（GPUI 的任务走 GCD 主队列，菜单跟踪的嵌套 run loop 里照常执行） |
| 图标 | `NSImage::imageWithSystemSymbolName_accessibilityDescription("chart.bar.xaxis", ..)` 加 `setTemplate(true)`，深浅色菜单栏自动适配，debug 二进制也不需要资源文件；`setAutosaveName` 让系统记住用户拖到的位置 |
| 取 NSWindow | `HasWindowHandle::window_handle(window)` 要写全限定：GPUI 的 `Window` 有同名的固有方法（返回 `AnyWindowHandle`）。AppKit 句柄给的是 content view，再取它的 `window()`。测试平台的窗口没有句柄（`NotSupported`） |
| 隐藏 | `orderOut:` 这一个窗口，不用 `App::hide()`（隐藏整个应用，Sparkle 的窗口也会被藏起来）；再 `setActivationPolicy(Accessory)` 去掉 Dock 图标。重开：`Regular` → `cx.activate(true)` → `window.activate_window()`，试验 10/10 稳定，不需要延后一帧。`cx.activate(true)` 是 `activateIgnoringOtherApps:`，前台是游戏时会被拒绝；用户点了菜单项或从 Finder 再次打开时，AppKit 回调里先 `NSApp.activate()`（`resident::bring_forward`，协作式激活认用户刚做的操作）。`setActivationPolicy:` 设成已有的值会返回 NO，先比一下再设 |
| **不能在 GPUI 的 update 里同步触发 `windowShouldClose:`** | 在 action、事件处理、`update` 里调 `NSWindow performClose:`，AppKit 同步回调 `windowShouldClose:`，GPUI 的包装借不到 `App`，回调返回 true，接着 `close_window` 再借用时 panic，进程 abort。所以 `⌘W` 绑自己的 `CloseWindow` action，直接调 `close_requested`；用户点红色按钮不受影响（那时 GPUI 不在 update 里）。同理，打开菜单（`popUpMenuPositioningItem`、`performClick:`）是嵌套 run loop，要经 `performSelector:withObject:afterDelay:0` 推到 run loop 上 |
| 测试 | `resident::install` 只在 `main()` 里调用，测试里没有 `Resident` 全局，`cx.try_global::<Resident>()` 为 `None`，平台调用都跳过；无界面测试只看 `window_hidden`、焦点和 `resident_view`（推给菜单栏项的内容） |

菜单栏管理工具（这台机器上是 Thaw）会把新的菜单栏项收进隐藏区（屏幕外），要用户在工具里拖到可见区；脚本不能指望点到它（见下文 `screenshot-resident`）。

## 自绘图表：`Plot`

Usage 视图的图（`usage_chart.rs`）没有用 gpui-component 现成的 `BarChart` / `LineChart`：它们画不了按模型堆叠、前一天的虚线框、移动平均和现在线叠在一张图上。做法是实现 `gpui_kit::component::plot::Plot`，在 `paint` 里用库提供的零件（`Grid`、`PlotAxis`、`shape::Bar`、`shape::Line`、`label::PlotLabel`）和 `window.paint_quad` / `paint_path` 自己画。

| 要点 | 做法 |
|---|---|
| 成为元素 | `#[derive(IntoPlot)]`，作用域里要有 `Plot` trait。生成的元素样式是 `size: full`，填满父元素：高度由外面的 `div().h(..)` 定（`CHART_H`） |
| 几何 | 全部放进纯函数 `Layout`（绘图区、每根柱子的位置、`slot_at(x)`、`y(value)`），先写测试（`usage_chart::tests`）。画图、提示框、点击都只经它算位置，三者不会对不上是哪根柱子。柱子再窄也不能宽过自己的槽：最小 3px 的柱宽在长的 Custom 范围（槽宽不到 3px）里会伸进隔壁那天，画出来的柱子点下去是另一天（审阅 R4），所以最小宽度取 `min(3, 槽宽)` |
| `PlotAxis` | `x_label` / `y_label` 在调用那一刻按已设的 `x(..)` / `y(..)` 和 `*_label_side` 算位置：先 `.x(..)`、`.y(..)`、`.y_label_side(..)`，再给标签，否则标签不显示或在错的一边。`y_axis(false)` 只画标签不画竖线 |
| 字体 | `PlotLabel` / `AxisText` 用当前文本样式的字体（`window.text_style()`），也就是外层 div 的 `font_family`；`font_weight` 被忽略，要加粗只能换字体。图的外层 div 设等宽字体，提示框自己设回无衬线 |
| 堆叠柱 | `shape::Bar` 一次画一组同样圆角的矩形：顶段（圆角 3px）和其余段（方角，段间留 1.5px）分两次画 |
| 虚线 | 线用 `PathBuilder::stroke(..).dash_array(&[..])`（均线、现在线）；虚线框用 `outline(bounds, color, BorderStyle::Dashed).corner_radii(..)` 画一个 quad，比拼四条虚线路径简单，圆角也对 |
| 悬停提示框 | `id()` 返回 `Some` 才开启。`tooltip_state(position, ..)` 的 `position` 相对图的左上角，返回 `TooltipState`（这里用 `slot_at` 定柱子）；`tooltip(..)` 返回任意元素，`plot::tooltip::Tooltip` 是带浮层定位的外壳，`CrossLine::band(..)` 给所在列加浅色底。悬停状态存在元素内部（每帧按 hitbox 重算），外面读不到也设不了。`Tooltip::new(cursor, within)` 把框放在 `cursor` 旁边：`cursor` 在上半部时框往下开，在下半部时往上开，框比光标以上的空间高就伸出图外、盖住图上方的标题行和图例（实机验收看到过）。所以只把鼠标的 x 传进去，y 固定为绘图区顶端减去 `gap`：框的顶边就是绘图区的顶边，比图高时往下伸进下一块区域，不往上 |
| 不靠鼠标显示提示框 | `Plot::prepaint` 可以返回额外的子元素：鼠标不在图上时，在这里按 `UsageState::hover` 造一个提示框并 `prepaint_as_root`，autoplay 截图和测试就能「钉住」某根柱子的提示框 |
| 点击 | `Plot` 没有点击回调。图外面套一层 div，`on_mouse_down` 交给 `AppRoot::click_chart`；图在 `paint` 里把自己的 bounds 写进 `Rc<Cell<Bounds<Pixels>>>`（`UsageState::chart_bounds`），点击处理用同一个 `Layout::slot_at` 换算。Plot 的 hitbox 是 `HitboxBehavior::Normal`，不挡外层 div 的鼠标事件 |
| 测试 | 无界面测试窗口照常布局和绘制：`VisualTestContext::simulate_mouse_move` / `simulate_click` 按坐标操作（坐标从记下的 bounds 算），提示框用 `.debug_selector(..)` 标记、`cx.debug_bounds(..)` 查有没有画出来、画在哪（`keyboard_tests.rs` 的 chart 测试）。同样的办法查布局：`simulate_resize` 把窗口改窄，再看 `debug_bounds` 是否还在窗口里（Usage 工具栏换行的测试；窄于 960px 时指标卡排成三张加两张）。fixture 的提示框比真实数据的矮，测提示框位置要把鼠标放在会让框往上开的地方（图的中线以下） |
| 纵轴刻度 | 小数位数跟着步长走（`usage_view.rs::fmt_axis`）：一天的 Haiku 只有零点几美分，固定两位小数会写出一串 `$0.00`。token 的步长只取 1、2、5 × 10ⁿ（不取 2.5k，`fmt_tokens` 写不出 2.5k），且至少为 1 |

## 测试

界面层的测试大部分是纯逻辑，不起窗口：行转换、过滤、排序、视图状态读写、store 与搜索线程的事件序列。

键盘路径是例外。`keyboard_tests.rs` 在无界面的测试窗口里把整个应用跑起来（数据是 core 的 fixture，索引和视图状态在临时目录），按键走 GPUI 自己的分发：同一套绑定、同一套 key context、同一套处理者。不发系统级按键，所以不会打进别的应用，也不需要屏幕。

| 要点 | 说明 |
|---|---|
| 依赖 | `[dev-dependencies]` 里给 `gpui-kit` 开 `test-support`，版本同样锁 `=0.6.1` |
| 测试属性 | `#[gpui_kit::test]`，参数是 `&mut TestAppContext` |
| 导入 | 逐个导入用到的名字，不写 `use gpui_kit::*` |
| 操作 | `gpui_kit::test::TestWindowExt`：`press("cmd-f")`、`input("文字")`、`click(id)` |
| 后台线程 | 先调 `cx.executor().allow_parking()`。测试调度器默认把别的线程发来的唤醒当成"测试不确定"而报错，而 store 和搜索线程是真线程 |
| 等后台结果 | 真实时间轮询：`run_until_parked()` 后检查条件，不满足就睡 10ms 再看。`wait_for` 走的是测试时钟，等不到真线程 |
| 定时器 | 防抖这类定时器走测试时钟，用 `cx.executor().advance_clock(..)` 推进。`cx.spawn` 里的 `timer(..)` 要等执行器第一次跑这个任务时才开始计时：输入之后先 `run_until_parked` 再 `advance_clock`，否则时钟推进了计时器还没建，它会在后面某次推进时钟时才触发（2026-11-16 一个审阅复现用例就因此在推进 30 秒时多跑了一次搜索，把表格滚回顶部）。按 Enter 立即搜索不会取消已排上的防抖搜索 |
| 计时 | 要和定时器一起被 `advance_clock` 推进的时刻（自动扫描的 30 秒节流），记 `cx.background_executor().now()`，不用 `Instant::now()` |
| 窗口激活 | 测试窗口创建时不是激活的。`window.activate_window()` 让它激活，`observe_window_activation` 的回调照常触发；要模拟「切走再回来」，再开一个窗口（`cx.add_window(\|_, _\| gpui_kit::EmptyView)`）激活它，再激活回来 |
| 剪贴板 | 测试平台有自己的剪贴板：应用里 `cx.write_to_clipboard(..)` 写进去，测试里 `cx.read_from_clipboard()` 读出来，不碰系统剪贴板 |
| 保存 / 打开对话框 | `cx.simulate_new_path_selection(..)` / `simulate_path_prompt_response(..)` 回答；测试平台的 `reveal_path` 是 `unimplemented!()`，测试里不要走到它 |
| 系统对话框（`window.prompt`） | `cx.has_pending_prompt()` 看有没有，`cx.simulate_prompt_answer("按钮文字")` 回答，再 `run_until_parked` 让等答案的任务跑完 |
| 点红色按钮 | `gpui_kit::VisualTestContext::from_window(window, cx).simulate_close()`：调注册的 `on_window_should_close` 回调，返回窗口是否真的关了 |
| 退出 | 测试平台的 `quit` 什么都不做：要断言「退出了」，看应用自己记下的标志（`AppRoot::quitting`） |
| 鼠标点击 | 给要点的元素挂 `.debug_selector(..)`（列表里只给第一行挂），测试里 `VisualTestContext::from_window(window, cx)`，`simulate_resize` 把窗口放大到元素在视口里，`run_until_parked` 画一帧，`debug_bounds(selector)` 取位置，`simulate_click(bounds.center(), Modifiers::none())`。走的是真实的命中测试和事件冒泡，能证明「点 `↗` 不会顺带点了行」这类问题（`keyboard_tests.rs` 的 `click_on`） |
| 按键之后断言焦点 | `press("cmd-f")` 把焦点放进搜索框后，紧接着在一次 update 里 `focus_table` 再 `run_until_parked`，焦点有时会回到搜索框，复不复现随编译结果变（同一个用例连跑 6 次都失败，按键后加一个 `run_until_parked` 连过 3 次，再加一行注释又连败 4 次；`focus_search` 只被调了一次，原因没查到）。测试只是要「焦点先在别处」时，在 `in_window` 里直接调 `focus_search`，不经按键（`clearing_the_active_tag_keeps_the_other_filters_and_the_focus`） |
| 动画与真实时钟 | GPUI 的 `with_animation` 按真实时间（`Instant`）走，`advance_clock` 推不动它。通知从上方滑进来（400ms），刚推出来的那一帧它的位置偏上 96px，盖不住它最终会盖的地方。测「通知盖没盖住某处」要先真实时间里一帧帧画过 0.6 秒（`settle_animations`：`run_until_parked`、`window.refresh()`、睡 10ms，循环）；通知的自动消失走测试时钟，不推就不会消失 |
| 窄窗口布局 | `simulate_resize` 到 1440 / 900 / 700，查每个控件的 `debug_bounds` 都在窗口里、搜索框不窄于 160；数不同的中心 y 得到行数（`the_session_toolbar_wraps_in_a_narrow_window_instead_of_running_off_it`）。指标卡的大数字是 `.self_start().whitespace_nowrap()`，元素宽就是数字宽，测试才看得出它有没有伸出卡片（拉伸到卡宽的元素永远「在卡里」） |
| 测滚动位置 | 测试窗口照常布局和绘制，`list` 会把滚过头的位置拉回来：fixture 的对话很短，整个放得下时 `scroll_to` 到第二张卡，下一帧就回到顶端。先 `cx.simulate_window_resize(window, size(..))` 把窗口压矮（`brackets_*` 用 300px 高） |

这类测试能证明按键到动作到状态的整条路径，证明不了系统把物理按键交给 GPUI 的那一段，也不看像素。

`use gpui_kit::*` 会带进 GPUI 自己的 `test` 属性宏，盖住标准库的。测试模块里要显式写：

```rust
use core::prelude::v1::test;
```

否则 `#[test]` 展开时递归超限，报错信息看不出和导入有关。

## 脚本化交互与截图

现有八个截图脚本（`scripts/screenshot*.sh`），每一张截的是什么写在各脚本的文件头和步骤注释里：

| 命令 | 驱动方式 | 截图 | 可调变量 |
|---|---|---|---|
| `make screenshot` | `ASM_SPIKE_AUTOPLAY=1` | 3 张：初始、自动滚动后、自动过滤加排序后 | 无 |
| `make screenshot-search` | `ASM_SEARCH_AUTOPLAY=1` | 7 张：文本搜索、下一个命中、两字 CJK、正则、Tools 范围、按 Matches 排序、清空 | `ASM_AUTOPLAY_QUERY` / `ASM_AUTOPLAY_CJK` / `ASM_AUTOPLAY_REGEX` 换查询 |
| `make screenshot-detail` | `ASM_DETAIL_AUTOPLAY=1` | 最多 12 张：选中、展开步骤、展开一步、全部展开、搜索命中、下一个命中、特殊回合、70MB 文件、History 行（第一个 prompt 不少于 3 条的）、代码块（展开离开头最近的带代码块的回复）、从顶部按两次 `]`、Metadata | `ASM_AUTOPLAY_SESSION` / `ASM_AUTOPLAY_REGEX` / `ASM_AUTOPLAY_SPECIAL` 指定 session 和查询；`ASM_AUTOPLAY_STEP` 取 `cut-output`、`thinking` 或 `mcp`，决定展开哪种步骤 |
| `make screenshot-resident` | `ASM_RESIDENT_AUTOPLAY=<gate 目录>` | 12 张：启动后（窗口、菜单栏条、Dock 条）、关窗后（菜单栏条、Dock 条）、弹出的菜单（按窗口 ID 截菜单本身）、窗口隐藏时 Scan Now 之后（菜单栏条、Dock 条）、再弹一次菜单、Open Sessions 之后（窗口、菜单栏条、Dock 条）；最后经菜单 Quit。每一步都有判定，任何一项不过退出码就是 1（见下文） | 必须设 `ASM_DB`、`ASM_CLAUDE_DIR`（指向副本）；`RESIDENT_BEFORE_SCAN` 是隐藏期间、Scan Now 之前执行的命令；`RESIDENT_HOLD=<秒>` 让窗口在那之后多隐藏一段时间，期间打出每次 `auto-scan: timer` 的时刻和间隔（超过 660 秒算不过），并加截 `2b-held-*`；`ASM_RESIDENT_QUERY=<词>` 在第一张截图前先搜索，1-shown 与 6-open 应显示同样的搜索词、结果和选中；`RESIDENT_FRONT_APP=<应用>`（如 Finder）在第二次弹菜单和 Open Sessions 之前用 `open -a` 把它带到前台；`RESIDENT_CLICK=1` 用 cliclick 真点菜单里的 Scan Now 和 Open Sessions；`RESIDENT_REOPEN=<.app>` 改用 `open` 再次打开应用（Finder 的路径，二进制必须是这个 .app 里的）；`RESIDENT_NOTICE=1` 从没显示过第一次关窗说明的状态开始；`ASM_RESIDENT_MENU_AT=x,y` 改菜单弹出的位置；`RESIDENT_SETTLE` 截图前等待的秒数 |
| `make screenshot-cost` | `ASM_COST_AUTOPLAY=1` | 最多 9 张：Opus 5.5 按 Ctx 降序比 Fable 5.2、换 Sonnet 5.5、有其他模型的 session 比 Haiku 5.5、按 In 排序、按 Hit% 升序、指定 session 的 Conversation 顶部与第一处缓存失效、它的 Metadata、隐藏 Started / Size / Kind | `ASM_AUTOPLAY_CACHE_SESSION` 指定看缓存失效的 session，没给就跳过那 3 张 |
| `make screenshot-usage` | `ASM_USAGE_AUTOPLAY=<gate 目录>` | 3–4 张：启动后用量读完（`1-ready`）、Scan for Changes 之后、Rebuild Index 之后，可选再开着 `USAGE_HOLD` 秒后一张；每张的状态栏都有 `Today $X` | 必须设 `ASM_DB`、`ASM_CLAUDE_DIR`（指向副本，否则拒绝运行）；`USAGE_BEFORE_SCAN` 是扫描前执行的 shell 命令，`USAGE_HOLD` 是最后开着的秒数 |
| `make screenshot-usage-view` | `ASM_USAGE_VIEW_AUTOPLAY=<gate 目录>` | 18 张：Today 按小时、钉住最贵一小时的提示框、Yesterday、30 天（Cost / In / Hit rate）、最贵一天的提示框、点进那一天、回到 30 天、选最贵的项目、This month、900px 窄窗口、切回 Sessions（项目过滤保留）、从 Usage 点这个项目的 `↗`（会话表带 `Active …` 标签）、从 Usage 点费用最高且会话表里有的 session（选中、详情、焦点在表格）、会话表 700px 窄窗口（工具栏换行，`Active …` 标签还在）、没有调用的范围；再用空数据目录启动一次截「No usage recorded yet」。每步都有判定，不过退出码是 1 | 必须设 `ASM_DB`（不能在真实数据目录，账本是它旁边的 `usage-ledger.sqlite`）；`ASM_CLAUDE_DIR` 可以是真实的（只读）或副本；`ASM_USAGE_VIEW_STEPS=a,b` 只跑其中几步（名字见脚本头）；`USAGE_VIEW_FRESH=0` 不做第二次启动 |
| `make screenshot-clicks` | cliclick 真点击，启动三次 | 22 张：1–11 是下拉菜单、过滤、Preview 模式、选中行和 Metadata tab，12–15 是自定义区间与日期选择器，16–20 是回合卡（第二次启动，预选 `5f6eea99`），21–22 是普通行和 History 行的右键菜单（第三次启动） | 坐标写在脚本里；换机器要换一个不再变化的 session 并重量坐标 |

脚本都先 `wake-screen`，再用 `caffeinate` 让显示器在 app 运行期间不睡（中途睡着会截不到图）。除 `screenshot-usage`、`screenshot-resident`、`screenshot-usage-view` 外都跑在真实索引上，用一次性的 `state.json` 保证从默认视图开始，按窗口 ID 截图，输出到 `tmp/<date>-screenshots/`（`SHOT_DIR=` 可改）。autoplay 的步骤做不了时会跳过不截。`screenshot-cost.sh` 用 `winid --pid` 只找自己启动的窗口，用户开着的同名 app 不受影响。

需要自动驱动界面时，优先加环境变量控制的内置脚本（`ASM_SPIKE_AUTOPLAY=1`：启动后滚动并选中，再切过滤、排序和 tab）。它走的是和用户操作相同的代码路径，不依赖屏幕坐标。

下拉菜单这类弹出层目前没有找到程序化打开的办法，只能用 cliclick 真点击。点击坐标是相对窗口的固定值，任何让控件移位的布局改动都要重新校对；工具栏按钮的位置取决于左边按钮的标签长度（选了项目、出现日期选择器都会把右边的按钮推开）。脚本启动的窗口不在前台，第一次点击只会激活窗口；GPUI 按上一帧的悬停状态判定点中了什么，指针瞬移过去立刻点击可能落空。所以 `screenshot-clicks.sh` 先点一下标题栏，每次点击前先把指针移过去停 200ms。详情面板的标题限制为单行也是这个原因：两行标题会把下面的 tab 和对话顶栏推下去 25px。表格行的右键菜单用 `cliclick rc:` 打开；History 行在表里的位置每天都变，脚本先用 Kind 下拉把列表收窄到 History，让它成为第一行再右键。导出的系统保存对话框不要用脚本点：app 从 agent 会话里启动时，对话框访问 iCloud Drive 会弹出记在 Claude Code 二进制名下的系统授权（2026-10-27 遇到），允许或拒绝都是用户的决定。

**不要用固定的 `sleep` 等界面就绪。** 新编译的二进制首次启动要几秒才出窗口，等不够就截到空窗口或上一帧的画面；`set -euo pipefail` 下这类失败经常没有任何输出，看起来就像脚本正常跑完了。做法是让 app 在每一步稳定后打一行标记（`autoplay-search: <step>`），脚本轮询窗口 id 和日志里的标记，超时就报错并指向 app 日志。`screenshot-search.sh` 是这个写法的参考；`screenshot-detail.sh`（`ASM_DETAIL_AUTOPLAY`，详情面板的 12 步）同样写法，没有可做的步骤打 `autoplay-detail: <step> skipped`，脚本跳过不截。

**`screenshot-resident` 的步骤也由文件驱动，顺序由脚本定。** autoplay 启动读完用量后打 `autoplay-resident: shown · <标题> | <三行> · status bar <Today>`，之后按序号执行脚本放进 gate 目录的命令文件（`1-close`、`2-menu`、`3-scan`…），每条做完打 `autoplay-resident: <命令> · …`：`close`（红色按钮的路径）、`menu`（在固定位置弹出菜单栏项的菜单，打印左上角坐标）、`close-menu`、`scan`（先关菜单，再经菜单项触发 Scan Now，等扫描结束）、`wait-scan`（等脚本真点击触发的扫描）、`open`（经菜单项 Open Sessions，打印窗口是否激活、表格是否有焦点）、`wait-open`（等脚本真点击或 `open` 引起的重开，打印同样的内容）、`quit`。菜单项经 `performActionForItemAtIndex:` 触发，走的是 target → 通道 → GPUI 任务这条真实路径，但不算用户的点击：macOS 可能因此不把应用带到前台（前台是游戏时就这样），要验点击就用 `RESIDENT_CLICK=1`。菜单不是点菜单栏项弹出的，因为 Thaw 把菜单栏项收在屏幕外；菜单栏条的截图里也就看不到它，它的标题以日志为准。脚本把应用带到前台两次（启动、Open Sessions），这期间敲的键会进应用的窗口。

**判定。** 脚本按日志和窗口列表判每一步：标题等于状态栏 Today 取整；关窗后 `hidden=true`、窗口不在屏幕上、进程还在；扫描结束且（有 `RESIDENT_BEFORE_SCAN` 时）Today 变了；截菜单时菜单还开着、窗口列表里有本进程的菜单窗口（`winid --menu`）；重开后 `shown=true active=true table-focused=true`；Quit 后进程退出且退出码为 0。不过的项当场打印 `FAILED: …` 并继续收集证据，最后列出全部并退出 1；没法继续的（窗口没出现、关窗没隐藏、应用中途退出）立即退出 1。从第一次关窗说明退出，只有日志有 `window: first-close notice · 立即退出` 且退出码为 0 才算通过。判定本身有行为测试：`make test-screenshot-resident`（`scripts/test-screenshot-resident.sh`，用假的应用、`winid`、`screencapture`、`cliclick` 演 13 种情形，不起 GUI、不截屏、不点击）。

**点击前的确认。** `RESIDENT_CLICK=1` 每次点击前用 `winid --at x,y`（`CGWindowListCopyWindowInfo` 里点击处最上层的窗口）确认那里是本进程的窗口（菜单），并且日志表明菜单开着；否则不点，记一项不过，改用程序化的选择。有的 Mac 上 Dock 有一个盖满整个屏幕的窗口（层级 20，`kCGDockWindowLevel`，不透明度 1），点击会穿过它，只有 Dock 本身那一条接点击；`winid --at` 原来对任何坐标都答 Dock（usage-p3-accept 踩过），现在在屏幕的可见区域（`NSScreen.visibleFrame`，不含菜单栏和 Dock 条）里跳过这个窗口，在 Dock 条上照旧答 Dock。别的层级照常算数：别的应用的浮动面板、工具窗口在层级 3–19，确实会接住点击（usage-p3-accept 的 `topwin.py` 跳过 1–23 全部层级，会把这种点击放过去）。菜单项的位置是相对菜单左上角量出来的：2026-11-16 加了 Open Usage 后在 `3-menu.png` 上重量，Open Sessions +101、Open Usage +125、Scan Now +148（行中心），脚本用 100 和 148，两次真点击都选中了对的项。菜单再加减项要重量。

**agent-desktop 能做什么**（usage-p3-accept 2026-11-16 实测，证据在 `tmp/2026-11-12-usage-stats/usage-p3-accept/ad-*.json`）。它走辅助功能接口，默认无头、不动光标，可以和别的验收同时跑；但 GPUI 自绘的内容几乎没有辅助功能元素，所以用处有限：

| 对象 | 结果 |
|---|---|
| 找应用 | `list-apps` 里 debug 二进制的名字是 `agent-session-manager`；`--app "Agent Session Manager"` 报 `APP_NOT_FOUND`（.app 包没试过） |
| 应用不在前台 | `list-windows` 给 `accessible: false`，窗口的 `snapshot` 报 `ACTION_NOT_SUPPORTED … window_without_accessibility_element`，只剩截图和坐标输入 |
| 主菜单栏（`--surface menubar`） | 随时能读到整棵菜单（106 个元素，含 View 里的 Sessions / Usage / Find / Preview…）。应用不在前台时项全是 `disabled`，`click` 超时；在前台时 `find --surface menubar --role menuitem --name Usage --exact` 再 `click @ref` 走 `AXPress`，无头成功（日志 `view: Usage`），光标不动 |
| 窗口里（应用在前台） | Sessions 视图有 43 个元素：搜索框（`Search sessions`）、表格行（无文字）、详情的三个 tab（`Metadata` / `Conversation` / `Cost`）、窗口按钮，其余按钮没有名字。Usage 视图只有窗口、一个 group、5 个无名按钮和窗口按钮：范围按钮、下拉、柱子、项目行、指标卡、状态栏都没有元素 |
| 菜单栏状态项（Open Usage / Scan Now） | `--surface menu-bar-extras` 在这台机器上报 `PLATFORM_NOT_SUPPORTED`，无头验不了 |

结论：能交给 agent-desktop 的只有主菜单栏的项（切视图、Find、Preview 模式等）和详情 tab，而且要先把应用带到前台，「不抢焦点」做不到。Usage 视图里的操作（范围、过滤、悬停、点柱子）用 autoplay，非真点不可时用 cliclick，并照 brief 在每次点击前确认前台应用和点击处的窗口。真点击前让指针在控件上停够（实测 ✕ 和 `Custom…` 第一下常落空，停 0.8 秒再点才成，见上文「指针瞬移过去立刻点击可能落空」）。

**菜单会自己关。** 应用不在前台时程序化弹出的菜单，在前台是游戏时不到一秒就关了（usage-p2-accept：截图里只有菜单下面的窗口），前台是 Finder、终端时开着直到被关。日志 `menu bar: menu closed (open N s)` 给出菜单开了多久，判定会报出当时的前台应用。

需要副本数据的三个脚本（`screenshot-usage.sh`、`screenshot-resident.sh`、`screenshot-usage-view.sh`）共用 `scripts/shot-lib.sh`：拒绝真实数据目录（`require_copies` 要求索引和记录目录都是副本；`require_db_copy` 只要求索引不在真实数据目录，Usage 视图只读记录文件）、编译 `winid`（`SHOT_WINID` 可换成测试用的替身）、按窗口 ID 或屏幕区域截图。两个脚本都按进程 ID 找窗口（`winid - --pid N`）：.app 里的窗口所有者名是 `Agent Session Manager`，不是二进制名，按名字找会找不到。`screencapture` 偶尔报 `could not create image from window` / `from rect`，同一张图稍后再截就成功，所以这两个截图函数失败时重试一次。`winid --screen` 打印主屏的宽高（点），用来截菜单栏条和 Dock 条（Dock 位置读 `defaults read com.apple.dock orientation`）。

**`screenshot-usage` 的步骤由文件驱动。** 用量验收要在两步之间改数据（删掉一半记录文件），所以 autoplay 不按固定间隔走：用量读完打 `autoplay-usage: ready · Today $X`，然后等 gate 目录里出现 `scan` 文件才做 Scan for Changes，扫描结束、表格重读后打 `autoplay-usage: scanned · …`；再等 `rebuild` 文件做 Rebuild Index，打 `autoplay-usage: rebuilt · …`。脚本在两步之间执行 `USAGE_BEFORE_SCAN`，然后 `touch` 对应的文件。之后 app 照常开着，自动扫描的日志行是 `auto-scan:`、`scan (automatic):`、`today:`。它会重建索引，所以脚本要求 `ASM_DB`、`ASM_CLAUDE_DIR` 都指向副本，`ASM_STATE` 默认在输出目录里新建；从 core 的 fixture 复制一份就能跑通全程（2026-11-16 自查过）。挪走记录文件用 `mv` 到别的目录，不必删除。

**`screenshot-usage-view` 是固定顺序的导览，不用 `ASM_USAGE_AUTOPLAY`。** 那个变量的 gate 驱动的是改数据的扫描和重建，`make screenshot-usage` 靠它；Usage 视图的导览只切视图状态，所以另用 `ASM_USAGE_VIEW_AUTOPLAY=<gate 目录>`（`usage_view.rs::usage_view_autoplay`）。每步做完打 `autoplay-usage-view: <step> · view=… range=… metric=… chart=hourly/24|daily/N hover=… empty=no|no-usage|no-calls|unavailable calls=… cost=… project=… width=… <状态栏 Today> today-matches=yes|no|-`，然后等 gate 目录里的 `<step>.shot` 再走下一步（最多 30 秒）；做不了的步骤打 `<step> skipped · 原因`，脚本记一项不过。行尾还有会话表一侧的状态：`active=<起>..<止>|- updated=any|limited rows=… selected=<短 id>|- detail=yes|no focus=table|search|usage|- usage-sessions=<Sessions 卡的数> toolbar=<行数>/inside|overflow|-`（最后一项是会话表工具栏上一帧的布局，`on_children_prepainted` 量的；Usage 下是 `-`）。提示框步骤设的是 `UsageState::hover`（钉住），不动鼠标；窄窗口步骤用 `window.resize` 改到 900px，截完恢复。两步跳转（第 4 期）放在 `sessions` 之后、`empty` 之前，前面 13 步的状态和截图不受影响：`jump-project` 在 Usage 里对费用最高的项目调 `show_active_sessions`（`↗` 的路径），把列出的 session id 打成 `jump-rows: <id>,<id>,…`，判定 `view=sessions`、Active 和 Project 都设上、`updated=any`、行数大于 0、焦点在表格，并且行数加上通知 `notice: 这段时间还有 N 个…` 里的 N 等于 Usage 的 Sessions 数；`jump-session` 回到 Usage，对「费用最高的 session」里第一个会话表有的调 `reveal_session`，判定选中的短 id 就是日志 `reveal: <短 id>` 那个、`detail=yes`、焦点在表格。之后 `sessions-narrow`（返工加的）把窗口改到 700px 截会话表，判定 `width=700`、Active 还在、`toolbar=` 多于一行且 `inside`，截完恢复宽度。`empty` 里 Active 过滤还开着，不影响 Usage。`jump-rows` 可以拿去和账本副本对：`usage_hourly` 里这个项目（`sessions.project_key`）在 `[起始日 4:00, 结束日次日 4:00)` 有 `responses > 0` 的小时桶的 session（`hour` 是 UTC 整点，按 `hour * 3600000` 比）。要看真实数字，把真实账本用只读方式备份到 `ASM_DB` 旁边：`sqlite3 "file:$HOME/Library/Application Support/dev.reorx.AgentSessionManager/usage-ledger.sqlite?immutable=1" ".backup <目录>/usage-ledger.sqlite"`（先确认没有 app 在写，WAL 为空；WAL 不空时在确认没有 app 在写之后，把 `usage-ledger.sqlite`、`-wal`、`-shm` 三个文件一起 `cp -c` 过去，副本打开时会把 WAL 并进去）；今天的记录文件用 `cp -c`（APFS 克隆，不占空间）复制到一个副本目录当 `ASM_CLAUDE_DIR`，扫描就不会碰真实目录。`~/.claude/projects` 可能是符号链接，`find` 要写成 `projects/`。

**脚本里"挑一个最有代表性的"要从当前可见的行里算，不要从全量数据里算。** 真实数据的分布很不均匀，按 session 数挑"最忙的项目"会稳定挑中某个工具批量产生的 history session，截出来的图说明不了任何功能。autoplay 选项目就是从过滤后的索引向量里取的。

**截图脚本跑出来的耗时不能当性能数字。** 屏幕睡着时 `wake-screen` 会把它唤醒，之后十几秒里查询耗时可以到平时的 4–8 倍（两字搜索 74–87ms 对 17–20ms，正则首次扫描 1.1–2.3s 对 0.26–0.31s）。量耗时用不起窗口的探针：`ASM_BENCH_DB=<索引副本> cargo test -p agent-session-manager real_index_timings -- --ignored --nocapture`，它和 debug app 是同一个构建配置。

启动时 stderr 里的 `error messaging the mach port for IMKCFRunLoopWakeUpReliable` 是 macOS 输入法框架的噪音，无影响。

## 已核实的 API

调研文档里标了"待确认"或写法不对的条目，以下是源码里的实际情况：

| API | 实际情况 |
|---|---|
| `h_resizable(id)` | 只接 id；状态可选，用 `.with_state(&Entity<ResizableState>)` 挂上 |
| `h_resizable` / `resizable_panel` / `ResizableState` | 在 `component` 根，`component::resizable::*` 是兼容路径 |
| `TabBar::on_click` | 回调签名 `Fn(&usize, &mut Window, &mut App)`，可直接传 `cx.listener(..)` |
| `StatusBar` | `.left(..)`、`.right(..)`，`.child(..)` 进中间区域 |
| `date_picker` / `calendar` | 在 `component::date_picker` / `component::calendar` |
| `DatePickerState::range(window, cx)` | 区间模式；`number_of_months`、`presets` 设在 `DatePicker` 元素上。只有起止两端都选好才发 `DatePickerEvent::Change`，第一下点击不发；快捷项直接发 `Change`。选完后它把焦点交给自己，订阅里要把焦点还给表格 |
| `cx.theme()` | 需要导入 `component::ActiveTheme` |
| `overflow_y_scrollbar()` | 在 `component::scroll::ScrollableElement` |
| `Window::viewport_size()` | 无参 |
| 输入框聚焦 | `InputState::focus_handle(cx)` 取句柄，再 `focus(window, cx)`；表格同理 |
| `InputEvent::PressEnter` | 有两个字段：`secondary` 和 `shift` |
| 输入框的 Escape | 是 `component::input::Escape` 这个 Action，在上层用 `.on_action(..)` 接 |
| `TableState::scroll_to_row` | 把目标行滚到顶部；`set_selected_row` 也会滚动 |
| 自绘滚动条 | 容器 `.overflow_y_scroll().track_scroll(&handle)`，外层 `.vertical_scrollbar(&handle)`；不需要句柄时直接 `.overflow_y_scrollbar()` |
| 剪贴板、Finder、保存对话框 | `cx.write_to_clipboard(ClipboardItem::new_string(..))`；`cx.reveal_path(path)` 在 Finder 里选中，`cx.open_with_system(dir)` 打开目录；`cx.prompt_for_new_path(dir, Some(文件名))` 返回 oneshot，结果是 `Result<Option<PathBuf>>`，取消是 `Ok(None)` |
| 带字段的 action | `#[derive(Clone, PartialEq, serde::Deserialize, gpui_kit::Action)]` 加 `#[action(namespace = app, no_json)]`；`KeyBinding::new("cmd-c", CopySessionId { id: None }, Some("DataTable"))` 直接绑一个值 |

## 相关文件

- `crates/app/src/main.rs` — `AppRoot`、事件接收、选中保持、Preview 列宽同步、窗口与菜单
- `crates/app/src/store.rs` — 后台线程与请求 / 事件类型
- `crates/app/src/keyboard_tests.rs` — 无界面窗口里的键盘测试，新写这类测试时照它的 `launch` / `wait_until` 写
- `crates/app/src/table.rs` — 表格 delegate
- `crates/app/src/style.rs` — 色值与主题安装
- `crates/app/src/resident.rs` — 菜单栏项、菜单的 target、隐藏窗口与 activation policy
- `crates/app/src/toolbar.rs` — 下拉菜单、搜索框与开关、搜索的防抖
- `crates/app/src/detail.rs` — 详情面板的标题与 Metadata，对话的状态（行表、展开 / 折叠、命中位置与停靠）
- `crates/app/src/turn_cards.rs` — 回合卡的绘制：`list` 的行、刻度的悬停与点击、命中的定位标记
- `crates/app/src/usage_view.rs` — Usage 视图：`UsageSnapshot`（图的数据 `ChartData`）、各面板、`usage_view_autoplay`
- `crates/app/src/usage_chart.rs` — Usage 的图：`Layout` 几何、`Plot` 实现、提示框、`click_chart`
- [GPUI 框架调研](gpui-framework-research.md) — 心智模型、组件目录、选型理由
- 设计稿链接与界面改版记录：`kb/notes/2026-09-16-design-refactor.md`
