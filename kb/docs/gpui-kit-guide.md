---
created: 2026-09-28
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
    ├── 工具栏        五个「标签 值 ▾」下拉 + 搜索框
    ├── h_resizable   左：DataTable    右：详情面板（可隐藏）
    ├── StatusBar     行数 · 索引状态 · 消息数与 DB 大小
    └── sheet / dialog / notification 层
```

快捷键和菜单项都是 Action：`gpui_kit::actions!` 定义，`cx.bind_keys` 绑定，`AppRoot` 的根元素上用 `.on_action(cx.listener(..))` 接收。

## 数据怎么进界面

界面不直接碰 SQLite，也不解析 JSONL。一个专门的后台线程独占数据库连接，界面通过消息和它往来：

```text
后台线程（独占 Db）                     主线程（AppRoot）
读索引 ──────────── Loaded ─────────▶ 整表替换
增量扫描 ── Progress / Session ─────▶ 状态栏进度 / 按 id 插入或替换
        ────────── ScanDone ────────▶ 状态栏摘要
扫描有改动则再读一次 ── Loaded ─────▶ 整表替换
请求 ◀── Scan / Rebuild / Messages(id)
```

- 主线程的接收任务每次醒来先取一条事件，再把积压的全部取走，一批只重算一次表格。冷启动会流进几千条 `Session` 事件，逐条重算会卡住界面
- 后台线程把所有错误转成 `Failed` 事件，单个请求失败不会带走线程
- core 类型到界面行只有一个转换点，所有格式化（时间、大小、预览截断）在那里一次做完，渲染时只取结果

**什么放后台**：只有 SQLite 查询和文件扫描。过滤和排序直接在主线程做，实测 10k 行排序 7ms、真实数据 5488 行过滤加排序 0–1ms，送去后台再传回来反而更慢更复杂。

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

选中按 session id 保持：数据或过滤变化后，按 id 重新找到该 session 的新位置。找不到就选第一行，表空则清空选中。

**流式扫描进行时不自动选中首行。** 设置选中行会顺带滚动到该行，而扫描期间首行一直在变，表格会跟着跳。启动时要恢复的选中也一样，等到那一行出现或扫描结束再定位。

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

## 元素函数的返回类型

edition 2024 下，`fn f(cx: &App) -> impl IntoElement` 的返回值会捕获 `cx` 的生命周期，同一个表达式里连续调用两次就借用冲突。带 `cx` 参数的辅助函数返回具体类型（`Div`、`AnyElement`、`StatusBar`），或者写 `-> impl IntoElement + use<>`。

## 异步任务

- `Task` 被 drop 就取消。`cx.spawn(..)` 的结果要么 `.detach()`，要么存进字段；存字段顺带得到取消能力
- 任务里更新视图的调用返回 `Result`，视图已销毁时失败。用 `.ok()` 或据此退出循环，不要 unwrap
- 闭包里持有视图用弱引用，避免循环引用

## 测试

界面层的测试都是纯逻辑，不起窗口：行转换、过滤、排序、视图状态读写、store 的事件序列。

`use gpui_kit::*` 会带进 GPUI 自己的 `test` 属性宏，盖住标准库的。测试模块里要显式写：

```rust
use core::prelude::v1::test;
```

否则 `#[test]` 展开时递归超限，报错信息看不出和导入有关。

## 脚本化交互与截图

需要自动驱动界面时，优先加环境变量控制的内置脚本（`ASM_SPIKE_AUTOPLAY=1`：启动后滚动并选中，再切过滤、排序和 tab）。它走的是和用户操作相同的代码路径，不依赖屏幕坐标。

下拉菜单这类弹出层目前没有找到程序化打开的办法，只能用 cliclick 真点击。点击坐标是相对窗口的固定值，任何让控件移位的布局改动都要重新校对。详情面板的标题限制为单行也是这个原因：两行标题会把下面的 tab 和 chip 推下去 25px。

**不要用固定的 `sleep` 等界面就绪。** 新编译的二进制首次启动要几秒才出窗口，等不够就截到空窗口或上一帧的画面；`set -euo pipefail` 下这类失败经常没有任何输出，看起来就像脚本正常跑完了。做法是让 app 在每一步稳定后打一行标记（`autoplay-search: <step>`），脚本轮询窗口 id 和日志里的标记，超时就报错并指向 app 日志。`screenshot-search.sh` 是这个写法的参考。

**脚本里"挑一个最有代表性的"要从当前可见的行里算，不要从全量数据里算。** 真实数据的分布很不均匀，按 session 数挑"最忙的项目"会稳定挑中某个工具批量产生的 history session，截出来的图说明不了任何功能。autoplay 选项目就是从过滤后的索引向量里取的。

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
| `cx.theme()` | 需要导入 `component::ActiveTheme` |
| `overflow_y_scrollbar()` | 在 `component::scroll::ScrollableElement` |
| `Window::viewport_size()` | 无参 |
| 输入框聚焦 | `InputState::focus_handle(cx)` 取句柄，再 `focus(window, cx)` |
| `TableState::scroll_to_row` | 把目标行滚到顶部；`set_selected_row` 也会滚动 |

## 相关文件

- `crates/app/src/main.rs` — `AppRoot`、事件接收、选中保持、Preview 列宽同步、窗口与菜单
- `crates/app/src/store.rs` — 后台线程与请求 / 事件类型
- `crates/app/src/table.rs` — 表格 delegate
- `crates/app/src/style.rs` — 色值与主题安装
- `crates/app/src/toolbar.rs`、`detail.rs` — 下拉菜单、详情面板
- [GPUI 框架调研](gpui-framework-research.md) — 心智模型、组件目录、选型理由
- 设计稿链接与界面改版记录：`kb/notes/2026-09-16-design-refactor.md`
