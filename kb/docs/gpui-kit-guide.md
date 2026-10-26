---
created: 2026-09-28
updated: 2026-10-27
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

应用只声明 `gpui-kit = "=0.6.1"` 一个 UI 依赖。它把 GPUI 和组件库一起重导出，再单独声明 `gpui` 或 `gpui-pre` 会让两份 GPUI 同时进入依赖图，类型互不相认。

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

### 排序状态

表头箭头的状态存在列定义里，而表格每次 `refresh` 都会向 delegate 重新要列定义。Preview 列宽同步会在窗口尺寸变化时触发 `refresh`，所以列定义必须始终反映当前排序，否则一拖窗口箭头就跳回默认列。

做法是以 delegate 自己记录的排序为准，每次排序后同步到列定义。表格自己的排序入口是私有的，程序化排序（恢复上次状态、autoplay）走 delegate 的 `set_sort`，之后调用 `refresh` 让箭头跟上。

### 选中行

`TableState::set_selected_row` 会顺带滚动到该行，没有只选中不滚动的入口。所以不能在行还在变动时反复设置选中，否则表格跟着跳。选中按 session id 保持以及由此而来的两个例外，规则在 [运行时架构](app-architecture.md) 的"选中"一节。

列选中默认是开的：点表头会选中整列，整列的高亮会盖掉行选中。本项目的表头点击只用于排序，所以建表时关掉了（`col_selectable(false)`）。

## 主题与密度

设计稿的全部色值集中在 `style.rs`，按名字取用。别处不写 hex，这样界面才能和设计稿逐项比对。

- **全局密度**靠把主题字号设成 12px。`Root` 每帧用主题字号设置 rem，组件内部以 rem 计的尺寸随之按比例缩小，正好落到设计稿的密度。需要精确尺寸的地方显式写 `px()`
- **改主题色要做三步**：改 `colors`；用 `colors` 重建 `tokens`（表格和状态栏读的是 `tokens`，只改 `colors` 它们看不到）；调用 `Theme::sync_base` 同步到行为层（滚动条等）
- **选中色必须半透明**：表格把选中行画成一个绝对定位的层，盖在单元格**上面**。不透明的颜色会把整行内容遮住。当前是强调色 16% 不透明度，叠在白底上约等于设计稿的 `#dde8fa`；选中边框设为透明以去掉 1px 描边

## 下拉菜单与按钮

工具栏的下拉是 `Button` 加 `.dropdown_menu(..)`，没有用 `Select`。

- 菜单内容的构造函数在每次打开时重新执行，所以直接捕获当前值来决定哪一项打勾即可，不需要维护菜单状态
- 菜单项用 `PopupMenuItem::element(..)` 放任意内容，`.checked(bool)` 打勾，回调里通过弱引用更新 `AppRoot`
- 靠右的下拉用 `dropdown_menu_with_anchor(Anchor::TopRight, ..)`，否则菜单会伸出窗口
- 按钮外观：`ButtonCustomVariant` 定底色和 hover 色，边框、高度、圆角直接链在 `Button` 上。实例样式最后生效，能覆盖变体
- 搜索框旁的开关（`.*`、`Aa`）也是自定义外观的 `Button`，开和关各用一套颜色，没有用 `Switch` 或 `ToggleGroup`。悬停说明用 `.tooltip(..)`

### 右键菜单与带参数的 action

表格行的右键菜单由 `TableDelegate::context_menu(row_ix, menu, ..)` 构造，每次打开时调用，`row_ix` 是右键点中的行（`TableState::right_clicked_row`），不是选中的行。菜单项派发的 action 从表格的焦点向上冒泡，到达 `AppRoot` 的 `on_action`。

- **菜单右侧的快捷键提示是按 action 相等去查绑定的**（`popup_menu.rs::render_key_binding`）。本项目的 action 带 `id: Option<String>`：菜单项派发 `Some(id)`，快捷键绑定的是 `None`，两者不相等，所以普通的 `menu(..)` 项不会显示 `⌘C`。要显示就自己画：`PopupMenuItem::element(|_, _| 左标签 + 右提示).action(Box::new(..)).disabled(..)`。核实过（M4b spec 风险第 3 条）：element 项支持 `disabled`，点击时照常派发 action 并关闭菜单，不需要退回 `menu_with_disabled`
- **element 项的外层 `h_flex` 没有 `min_w_0`**，内容里有一段很长的不换行文字（resume 命令预览）时，它会按整段文字撑宽，`truncate()` 不起作用，右侧提示被推到菜单外。给自绘内容一个固定宽度：菜单宽度减去菜单边距 4px 和项内边距 8px（两侧共 24px），见 `table.rs` 的 `MENU_ITEM_W`
- 普通项置灰用 `menu_with_disabled(label, action, disabled)`。菜单项没有悬停提示，置灰的原因只能写在项里（本项目把它写在 Copy Resume Command 的预览行）

### 按钮的图标与置灰

- **补图标**：`gpui_kit::assets::Assets` 只带组件库用到的 101 个图标。要别的 Lucide 图标，用 `gpui_kit::assets::icon_assets!(ExtraIcons, [SquareTerminal, ..])` 生成只含这几个的 `AssetSource`，再写一个自己的 `AssetSource`：先查 `ExtraIcons`，查不到回落到 `Assets`，`list` 合并去重（`main.rs` 的 `AppAssets`，照 gpui-kit-assets README 的写法）。变体名是文件名的驼峰形式（`file-search.svg` → `FileSearch`），`gpui_kit::assets::IconName` 里能直接用
- **图标尺寸**：`Button::icon(..)` 按按钮尺寸缩放图标，以 rem 计；本项目 rem 是 12px，小号按钮的图标只有 10px 左右。要画稿的 14px，就把 `Icon::new(..).with_size(px(14.))` 作为 child 放进按钮
- **置灰的按钮照样显示悬停提示。** gpui-base 的 `Button` 置灰时只拦 mouse down 和 click，悬停不拦，`tooltip(..)` 照常出现。所以标题行置灰的按钮把提示换成原因
- **置灰的外观**：自定义变体（`ButtonCustomVariant`）置灰时背景是底色的 15%、边框是底色的 15%（白底就看不见了）。实例上链的 `.border_color(..)` 在置灰状态下也会覆盖回来，再加 `.opacity(0.4)` 就是画稿的样子
- 带快捷键的按钮用 `tooltip_with_action(文字, &action, Some("DataTable"))`，提示里会画出键帽

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
| 定时器 | 防抖这类定时器走测试时钟，用 `cx.executor().advance_clock(..)` 推进 |
| 剪贴板 | 测试平台有自己的剪贴板：应用里 `cx.write_to_clipboard(..)` 写进去，测试里 `cx.read_from_clipboard()` 读出来，不碰系统剪贴板 |
| 保存 / 打开对话框 | `cx.simulate_new_path_selection(..)` / `simulate_path_prompt_response(..)` 回答；测试平台的 `reveal_path` 是 `unimplemented!()`，测试里不要走到它 |
| 测滚动位置 | 测试窗口照常布局和绘制，`list` 会把滚过头的位置拉回来：fixture 的对话很短，整个放得下时 `scroll_to` 到第二张卡，下一帧就回到顶端。先 `cx.simulate_window_resize(window, size(..))` 把窗口压矮（`brackets_*` 用 300px 高） |

这类测试能证明按键到动作到状态的整条路径，证明不了系统把物理按键交给 GPUI 的那一段，也不看像素。

`use gpui_kit::*` 会带进 GPUI 自己的 `test` 属性宏，盖住标准库的。测试模块里要显式写：

```rust
use core::prelude::v1::test;
```

否则 `#[test]` 展开时递归超限，报错信息看不出和导入有关。

## 脚本化交互与截图

需要自动驱动界面时，优先加环境变量控制的内置脚本（`ASM_SPIKE_AUTOPLAY=1`：启动后滚动并选中，再切过滤、排序和 tab）。它走的是和用户操作相同的代码路径，不依赖屏幕坐标。

下拉菜单这类弹出层目前没有找到程序化打开的办法，只能用 cliclick 真点击。点击坐标是相对窗口的固定值，任何让控件移位的布局改动都要重新校对；工具栏按钮的位置取决于左边按钮的标签长度（选了项目、出现日期选择器都会把右边的按钮推开）。脚本启动的窗口不在前台，第一次点击只会激活窗口；GPUI 按上一帧的悬停状态判定点中了什么，指针瞬移过去立刻点击可能落空。所以 `screenshot-clicks.sh` 先点一下标题栏，每次点击前先把指针移过去停 200ms。详情面板的标题限制为单行也是这个原因：两行标题会把下面的 tab 和对话顶栏推下去 25px。

**不要用固定的 `sleep` 等界面就绪。** 新编译的二进制首次启动要几秒才出窗口，等不够就截到空窗口或上一帧的画面；`set -euo pipefail` 下这类失败经常没有任何输出，看起来就像脚本正常跑完了。做法是让 app 在每一步稳定后打一行标记（`autoplay-search: <step>`），脚本轮询窗口 id 和日志里的标记，超时就报错并指向 app 日志。`screenshot-search.sh` 是这个写法的参考；`screenshot-detail.sh`（`ASM_DETAIL_AUTOPLAY`，详情面板的 12 步）同样写法，没有可做的步骤打 `autoplay-detail: <step> skipped`，脚本跳过不截。

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
- `crates/app/src/toolbar.rs` — 下拉菜单、搜索框与开关、搜索的防抖
- `crates/app/src/detail.rs` — 详情面板的标题与 Metadata，对话的状态（行表、展开 / 折叠、命中位置与停靠）
- `crates/app/src/turn_cards.rs` — 回合卡的绘制：`list` 的行、刻度的悬停与点击、命中的定位标记
- [GPUI 框架调研](gpui-framework-research.md) — 心智模型、组件目录、选型理由
- 设计稿链接与界面改版记录：`kb/notes/2026-09-16-design-refactor.md`
