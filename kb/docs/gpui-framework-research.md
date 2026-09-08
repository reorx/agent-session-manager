---
created: 2026-09-08
tags:
  - gpui
  - gpui-kit
  - rust
  - gui
  - research
---

# GPUI 框架调研：面向数据展示类桌面应用

调研日期 2026-09-08。目标：为 Agent Session Manager（以表格展示数千条 Claude Code session，带过滤、排序、搜索、详情面板）选定 GPUI 的使用方式，摸清与"数据展示类应用"相性最好的部分，并把坑提前列出来。

信息来源：crates.io API、gpui.rs、docs.rs、Zed 仓库 main 分支（commit `3384317a`，2026-09-08）、longbridge/gpui-kit 仓库（HEAD `294ba34`）、context7、本机两次试编译。所有 API 签名以 main 分支 / gpui-kit 0.6.0 为准；与 crates.io `gpui 0.2.2` 有差异处会标注。

---

## 1. 现状与版本选择（最重要的决策）

### 1.1 GPUI 本体的发布状态

| 事实 | 说明 |
|---|---|
| crates.io `gpui` 最新 **0.2.2**（2025-10-22） | 已 11 个月无新版本；Apache-2.0；edition 2024 |
| main 分支已大幅分化 | 平台层拆成 `gpui_platform`（macOS Metal / Linux wgpu / Windows / wasm），入口从 `Application::new()` 改为 `gpui_platform::application().run(..)`；`gpui_platform` **Zed 未发布到 crates.io** |
| 2026-02 Zed 维护者公开声明 | "GPUI development is getting some major brakes put on it… pushing off anything that isn't directly related to Zed's use case"（HN 47003569）。GPUI 仍在 Zed 内部演进，但面向第三方的发版、文档被降优先级 |
| Zed 工具链 | Rust **1.97.1**、wgpu 29、taffy、smol 2、postage |

因此拿到 main 分支 API 的现实途径：

| 途径 | 维护者 | 说明 |
|---|---|---|
| **`gpui-pre` 0.3.4**（2026-09-07） | Longbridge（huacnlee） | "snapshot of zed@6916400"，纯快照不带行为补丁；配套 `gpui-pre-platform`、`gpui-pre-macros` 等 |
| **`gpui-kit` 0.6.0**（2026-09-03） | Longbridge | 原 gpui-component，facade crate，重导出 gpui-pre + 组件库 + 资源 |
| `gpui-ce` 0.3.3 | 社区 fork（1k star） | 落后 Zed main 数百 commit，"mostly API compatible, but this is changing" |
| git 依赖 zed main | — | 需 Rust 1.97.1，构建最慢，锁文件受 Zed workspace 约束 |

### 1.2 本机试编译结论

| 尝试 | 结果 |
|---|---|
| `gpui = "0.2.2"` hello world | 712 个包，70s 后在 gpui build script 失败：`cannot execute tool 'metal' due to missing Metal Toolchain` |
| `gpui-pre = "0.3.4"` hello world | 667 个包，rustc 1.91 编译失败：`E0658 use of unstable library feature slice_as_array` |

**开工前必须做的环境准备**：

```bash
# 1. Rust: 用 rustup 安装最新 stable（Homebrew 1.91 太旧；Zed 用 1.97.1）；建议 mise 或 rustup 管理
# 2. Xcode 26 不再自带 Metal 编译器，二选一：
xcodebuild -downloadComponent MetalToolchain
#    或在 gpui-pre-platform / gpui-kit 上开启 runtime_shaders feature（运行时编译 MSL）
```

### 1.3 决策：用 `gpui-kit = "=0.6.0"`，精确锁版

理由：
- 唯一成熟、Apache-2.0、活跃维护（每天约 10 commit）的 GPUI 组件库；`DataTable`、`Input`、`Select`、`DatePicker`、`TabBar`、`h_resizable`、`TextView::markdown` 把本项目 UI 需求全覆盖。
- 自带主题系统（20+ JSON 主题、暗色跟随系统），省掉自写 Theme。
- Zed 自己的 `crates/ui`、`crates/theme` 是 **GPL-3.0-or-later 且 `publish = false`**，vendor 进来会传染许可证并拖入 Zed 的 settings 体系，不可取（只读源码学模式）。

代价与对策：
- API 每个 minor 都有 breaking rename（`Dropdown→Select`、`Modal→Dialog`、`Table→DataTable`、`is_eof→has_more`、`gpui→gpui-pre`…），0.5.1 到 0.6.0 隔了 7 个月然后一次性大改。**对策：`=0.6.0` 精确锁定，提交 `Cargo.lock`，升级作为独立任务。**
- Longbridge 单一供应商同时控制 kit 和 gpui-pre 快照。**对策：核心逻辑（解析、索引、搜索）放在不依赖 gpui 的独立 crate，UI 层薄。**
- `DataTable` 行高固定。**对策：见 §4.3。**

`Cargo.toml` 模板（来自 gpui-kit 官方 installation.md，字面复制）：

```toml
[dependencies]
gpui-kit = "=0.6.0"        # default features = ["component", "assets"]
anyhow = "1.0"

# 显著加快 debug 构建；必须写在根 Cargo.toml
[profile.dev.package]
gpui-pre = { opt-level = 3 }
gpui-component = { opt-level = 3 }
gpui-kit = { opt-level = 3 }
gpui-kit-assets = { opt-level = 3 }
gpui-pre-macros = { opt-level = 3 }
gpui-pre-platform = { opt-level = 3 }
rustybuzz = { opt-level = 3 }
taffy = { opt-level = 3 }
ttf-parser = { opt-level = 3 }
```

**不要自己再声明 `gpui` 依赖**，否则会出现两份 GPUI 导致类型不匹配。若某个辅助 crate 必须直接依赖，用 `gpui = { package = "gpui-pre", version = "=0.3.2" }` 与 gpui-kit 锁文件一致。gpui-kit 的 `lib.rs` 重导出：`pub use ::gpui::*; pub use ::gpui_base as base; pub use ::gpui_component as component; pub use ::gpui_kit_assets as assets;` 以及自己的 `actions!` 宏。

参考数据：纯 gpui hello world 约 4MB release 二进制，加组件库约 12MB；首次干净构建数分钟，增量 debug 约 1s。

---

## 2. 核心心智模型

### 2.1 四个上下文与 Entity

摘自 `crates/gpui/docs/contexts.md`：

- **`App`**：根上下文，拥有所有 entity 的数据。
- **`Context<T>`**：操作某个 `Entity<T>` 时拿到的上下文，deref 到 `App`，额外提供 `notify`、`emit`、`observe`、`subscribe`、`spawn`。
- **`AsyncApp` / `AsyncWindowContext`**：跨 `await` 持有的 `'static` 版本，所有调用变为 fallible（窗口/应用可能已销毁）。
- **`Entity<T>`**：有状态数据的句柄，数据由 `App` 拥有。`T: Render` 时俗称 view。

2025-01 大重命名（PR #22632）：`View/Model → Entity`、`AppContext → App`、`ModelContext → Context`、`WindowContext → (&mut Window, &mut App)`、`ViewContext<T> → (&mut Window, &mut Context<T>)`、`cx.new_view/new_model → cx.new`、`AsyncAppContext → AsyncApp`。**2025 年以前的博客全部过时。**

关键签名（main 分支）：

```rust
pub trait Render: 'static + Sized {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement;
}
pub trait RenderOnce: 'static {            // 无状态组件，消费 self；配合 #[derive(IntoElement)]
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement;
}
pub trait EventEmitter<E: Any>: 'static {}
pub trait Global: 'static {}

// Entity<T>
fn read<'a>(&self, cx: &'a App) -> &'a T
fn update<R>(&self, cx: &mut impl AppContext, f: impl FnOnce(&mut T, &mut Context<T>) -> R) -> R
fn downgrade(&self) -> WeakEntity<T>       // WeakEntity::update 返回 Result<R>

// Context<T>
fn notify(&mut self)
fn emit<Evt>(&mut self, event: Evt) where T: EventEmitter<Evt>
fn observe<W>(&mut self, e: &Entity<W>, f: impl FnMut(&mut T, Entity<W>, &mut Context<T>)) -> Subscription
fn subscribe<T2, Evt>(&mut self, e: &Entity<T2>, f: impl FnMut(&mut T, Entity<T2>, &Evt, &mut Context<T>)) -> Subscription
fn spawn<R>(&self, f: impl AsyncFnOnce(WeakEntity<T>, &mut AsyncApp) -> R + 'static) -> Task<R>
fn listener<E>(&self, f: impl Fn(&mut T, &E, &mut Window, &mut Context<T>)) -> impl Fn(&E, &mut Window, &mut App)
fn processor<E, R>(&self, f: impl Fn(&mut T, E, &mut Window, &mut Context<T>) -> R) -> impl Fn(E, &mut Window, &mut App) -> R

// App
fn background_executor(&self) -> &BackgroundExecutor
fn background_spawn<R>(&self, fut: impl Future<Output = R> + Send + 'static) -> Task<R>
fn global<G: Global>(&self) -> &G / set_global / update_global / observe_global
fn open_window<V: Render>(&mut self, opts: WindowOptions, build: impl FnOnce(&mut Window, &mut App) -> Entity<V>) -> Result<WindowHandle<V>>
fn bind_keys(&mut self, bindings: impl IntoIterator<Item = KeyBinding>)
fn on_action<A: Action>(&mut self, f: impl Fn(&A, &mut Self))
fn set_menus(&self, menus: impl IntoIterator<Item = Menu>)
```

### 2.2 数据流与重渲染

View 是 retained 的：只有被 `cx.notify()` 标脏的 entity 才会在下一帧重新 `render`，其所有 `observe` 订阅者同时触发；`emit`/`subscribe` 是 entity 之间的类型化 pub/sub。官方 `_ownership_and_data_flow.rs` 的示例：

```rust
gpui_platform::application().run(|cx: &mut App| {
    let first_counter: Entity<Counter> = cx.new(|_cx| Counter { count: 0 });
    let second_counter = cx.new(|cx: &mut Context<Counter>| {
        cx.observe(&first_counter, |second: &mut Counter, first: Entity<Counter>, cx| {
            second.count = first.read(cx).count * 2;
        })
        .detach();
        Counter { count: 0 }
    });
    first_counter.update(cx, |counter, cx| {
        counter.count += 1;
        cx.notify();
    });
    assert_eq!(second_counter.read(cx).count, 2);
});
```

对本项目的映射：`SessionStore`（Entity 或 Global）持有全量数据与索引句柄；`SessionTable` view 持有"过滤+排序后的索引向量 `Vec<usize>`"；过滤条件变化 → 后台计算新索引向量 → `table.update(cx, |t, cx| { t.rows = idx; cx.notify() })`。

### 2.3 Element 与样式

`div()` 返回 `Div`，`.id(..)` 后变 `Stateful<Div>` 才能 `on_click`、hover、滚动跟踪。样式是 Tailwind 风格的 `Styled` trait（115 个方法）：`flex() flex_1() gap_2() text_sm() truncate() whitespace_nowrap() overflow_hidden() bg() text_color() border_b_1() rounded_md() cursor_pointer() line_clamp()`。`FluentBuilder` 提供 `when(cond, |el| ..)`、`when_some(opt, |el, v| ..)`。不同分支返回不同具体类型时用 `.into_any_element()` 统一。

---

## 3. 大列表虚拟化：GPUI 与本项目相性最好的部分

### 3.1 `uniform_list`（固定行高，本项目主力）

源码 doc：*"A scrollable list of elements with uniform height, optimized for large lists. Rather than use the full taffy layout system, uniform_list simply measures the first element and then lays out all remaining elements in a line based on that measurement."*

```rust
pub fn uniform_list<R: IntoElement>(
    id: impl Into<ElementId>,
    item_count: usize,
    f: impl 'static + Fn(Range<usize>, &mut Window, &mut App) -> Vec<R>,
) -> UniformList
// .track_scroll(&UniformListScrollHandle)  ← main 分支取引用；0.2.2 是按值
// .with_sizing_behavior(ListSizingBehavior) .with_width_from_item(Option<usize>) .with_decoration(..)

impl UniformListScrollHandle {           // Rc<RefCell<..>>，clone 廉价，存在 view 字段里每帧传入
    fn scroll_to_item(&self, ix: usize, strategy: ScrollStrategy)   // Top | Center | Bottom | Nearest
    fn logical_scroll_top_index(&self) -> usize
    fn is_scrolled_to_end(&self) -> Option<bool>
}
```

每帧只对可见区间调用闭包，成本 O(可见行 × 单元格)；`item_count` 每帧重新读取，所以"重算索引向量 + notify"就是刷新。闭包是 `Fn + 'static`，要访问 `&mut Self` 用 `cx.processor(|this, range, window, cx| ..)`。

### 3.2 `list`（可变行高，详情面板消息流用）

*"…Clients of this API need to ensure that elements outside of the scrolled area do not change their height… notify the list element via `ListState::splice` or `ListState::reset`."*

```rust
pub fn list(state: ListState, render_item: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static) -> List
impl ListState {
    fn new(item_count: usize, alignment: ListAlignment, overdraw: Pixels) -> Self
    fn measure_all(self) -> Self
    fn reset(&self, element_count: usize)
    fn splice(&self, old_range: Range<usize>, count: usize)
    fn scroll_to_reveal_item(&self, ix: usize)
    fn set_follow_mode(&self, mode: FollowMode)    // 聊天式跟随尾部
}
```

内部用 SumTree 存测量高度，比 `uniform_list` 重；行数变化必须 `reset/splice`。适合详情面板里高度不一的消息列表（或 gpui-kit 的 `v_virtual_list`）。

### 3.3 官方 `examples/data_table.rs`：几乎就是本项目的骨架

10,000 行 × 24 列，表头 + `uniform_list` 主体 + 自绘滚动条。核心模式：

```rust
#[derive(IntoElement)]
struct TableRow { ix: usize, quote: Rc<Quote> }          // Rc 共享行数据，避免 clone
impl RenderOnce for TableRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div().flex().flex_row().border_b_1().border_color(rgb(0xE0E0E0))
            .bg(if self.ix.is_multiple_of(2) { rgb(0xFFFFFF) } else { rgb(0xFAFAFA) })
            .py_0p5().px_2()
            .children(FIELDS.map(|(key, width)| self.render_cell(key, px(width), color)))
    }
}

struct DataTable {
    quotes: Vec<Rc<Quote>>,
    visible_range: Range<usize>,
    scroll_handle: UniformListScrollHandle,
}
impl Render for DataTable {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_col().size_full()
            .child(/* header: FIELDS.map(|(key, w)| div().w(px(w)).flex_shrink_0().truncate().child(key)) */)
            .child(
                uniform_list("items", self.quotes.len(),
                    cx.processor(move |this, range: Range<usize>, _, _| {
                        this.visible_range = range.clone();
                        range.filter_map(|i| this.quotes.get(i).map(|q| TableRow::new(i, q.clone()))).collect()
                    }),
                )
                .size_full()
                .track_scroll(&self.scroll_handle),
            )
    }
}
```

列宽是 `const FIELDS: [(&str, f32); 24]`，表头与单元格共用，保证对齐。**这是自建表格的下限方案**：约 200 行代码即可拥有 10k 行流畅滚动，但排序、选中、列宽拖拽、键盘导航都要自己写（gpui-kit 的 `state.rs` 有 2,568 行做这些）。

### 3.4 Zed 自己的 `ui::Table`（只读源码学习）

`crates/ui/src/components/data_table.rs`：`.uniform_list(id, row_count, render)` 或 `.variable_row_height_list(row_count, ListState, render)`，支持 `pin_cols`、`striped`、`ColumnWidthConfig::{auto, explicit, redistributable, Resizable}`。无排序/选中/事件。GPL + 未发布，不可依赖。

---

## 4. gpui-kit 0.6.0 组件库

### 4.1 Crate 分层与初始化

```text
gpui-kit             应用唯一依赖的 facade
├── gpui-base        无样式的行为/状态/基础设施（TextView 等的规范实现在这）
└── gpui-component   完整的带样式 UI 系统
```

`main.rs`（官方 getting-started.md 字面复制）：

```rust
use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::*;

pub struct HelloWorld;
impl Render for HelloWorld {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().v_flex().gap_2().size_full().items_center().justify_center()
            .child("Hello, World!")
            .child(Button::new("ok").primary().label("Let's Go!")
                .on_click(|_, _, _| println!("Clicked!")))
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    app.run(move |cx| {
        gpui_kit::init(cx);            // 必须先于任何组件使用
        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|_| HelloWorld);
                cx.new(|cx| Root::new(view, window, cx))   // 窗口第一层必须是 Root
            }).expect("Failed to open window");
        }).detach();
    });
}
```

要点：
- **`Root` 必须是每个窗口的第一层 view**；顶层 render 末尾要 `.children(Root::render_dialog_layer(window, cx))`、`render_sheet_layer`、`render_notification_layer`。
- 图标来自 `gpui-kit-assets`（Lucide SVG），通过 `assets` feature + `.with_assets(..)`；自定义图标实现 `AssetSource`。
- 主题：`gpui_kit::init(cx)` 安装默认主题；`Theme::change(ThemeMode::Dark, Some(window), cx)`、`Theme::sync_system_appearance(..)`；取色 `use gpui_kit::component::ActiveTheme as _; cx.theme().primary / .muted_foreground / .border / .tokens.table`。
- Feature：`component`、`assets`、`test-support`、`inspector`、`profiler`、`tree-sitter`（Markdown 代码块高亮需要）。

### 4.2 `DataTable`：本项目的表格基座

```rust
use gpui_kit::component::table::{DataTable, TableState, TableDelegate, Column, ColumnSort, ColumnFixed, TableEvent};
```

`TableDelegate` 完整签名（`crates/component/src/table/delegate.rs`）：

```rust
pub trait TableDelegate: Sized + 'static {
    fn columns_count(&self, cx: &App) -> usize;                                   // 必需
    fn rows_count(&self, cx: &App) -> usize;                                      // 必需
    fn column(&self, col_ix: usize, cx: &App) -> Column;                          // 必需
    fn render_td(&mut self, row_ix: usize, col_ix: usize, window: &mut Window, cx: &mut Context<TableState<Self>>) -> impl IntoElement; // 必需
    fn perform_sort(&mut self, col_ix: usize, sort: ColumnSort, window: &mut Window, cx: &mut Context<TableState<Self>>) {}
    fn render_th(&mut self, col_ix: usize, ..) -> impl IntoElement { .. }
    fn render_tr(&mut self, row_ix: usize, ..) -> Stateful<Div> { div().id(("row", row_ix)) }
    fn context_menu(&mut self, row_ix: usize, menu: PopupMenu, ..) -> PopupMenu { menu }
    fn move_column(&mut self, col_ix: usize, to_ix: usize, ..) {}
    fn render_empty(&mut self, ..) -> impl IntoElement { .. }
    fn loading(&self, cx: &App) -> bool { false }
    fn render_loading(&mut self, size: Size, ..) -> impl IntoElement { Loading::new().size(size) }  // 内置骨架屏
    fn has_more(&self, cx: &App) -> bool { false }                                // 0.6 前叫 is_eof
    fn load_more_threshold(&self) -> usize { 20 }
    fn load_more(&mut self, ..) {}
    fn visible_rows_changed(&mut self, visible_range: Range<usize>, ..) {}        // 懒加载可见行的昂贵字段
    fn cell_text(&self, row_ix: usize, col_ix: usize, cx: &App) -> String { String::new() }  // CSV 导出用
}
```

`Column`：`Column::new(key, name).sortable().width(px(..)).text_right().fixed_left().resizable(bool).movable(bool).min_width(..)`；`ColumnSort { Default, Ascending, Descending }`。

`TableState<D>`：`TableState::new(delegate, window, cx)`；builder `row_selectable / col_selectable / cell_selectable / sortable / col_resizable / col_movable / loop_selection`；方法 `delegate() / delegate_mut() / refresh(cx)`（列变化后调用）、`scroll_to_row(ix, cx)`、`selected_row() / set_selected_row(ix, cx)`、`visible_range()`、`dump(cx)`（CSV）。

事件（字面）：

```rust
pub enum TableEvent {
    SelectRow(usize), DoubleClickedRow(usize), SelectColumn(usize), SelectCell(usize, usize),
    DoubleClickedCell(usize, usize), ColumnWidthsChanged(Vec<Pixels>), MoveColumn(usize, usize),
    RightClickedRow(Option<usize>), RightClickedCell(usize, usize), ClearSelection,
}
// cx.subscribe_in(&table, window, Self::on_table_event) 并保存 Subscription
```

元素：`DataTable::new(&state).stripe(true).bordered(true).small() / .with_size(px(48.))`。内置键位（key context `"DataTable"`）：上下左右、home/end、pageup/pagedown、escape、tab。

**内部实现**：行用 `gpui::uniform_list("table-uniform-list", ..)`，列用 `virtual_list` 横向虚拟化，表头叶子列也虚拟化。Story 默认 5,000 行 × 45 列，可切到 1,000,000 行并以 33ms 刷新前 2,000 行的 30%，源码注释记录 "561 cells: 232µs, avg 414ns / frame 8.8ms"。本项目几千行远在测试包络之内。

**刷新模式**（`data_table_story.rs` 字面）：

```rust
this._load_rows_task = cx.spawn(async move |this, cx| {
    let stocks = cx.background_spawn(async move { regenerate_stocks(size) }).await;
    this.update(cx, |this, cx| {
        this.table.update(cx, |table, cx| {
            table.delegate_mut().set_stocks(stocks);
            table.refresh(cx);
        });
        cx.notify();
    }).ok();
});
```

`rows_count()` 每次 render 重新读取，改 delegate 数据后 `cx.notify()` 即可；列定义变化才需要 `refresh(cx)`。

**排序模式**（story 字面）：`perform_sort` 里对 delegate 内的向量原地 `sort_by`，`ColumnSort::Default` 恢复自然序。

### 4.3 行高固定的限制与对策

`Size::table_row_height()`：XSmall 26px、Small 30px、Medium 32px（默认）、Large 40px、自定义 `with_size(px(n))`。单元格 `overflow_hidden`，多行会被裁掉。本项目"预览列"（首条 prompt / 最后消息）需要更多信息时：

1. **推荐**：`with_size(px(48.))` 两行布局：第一行标题（truncate），第二行灰色预览（truncate）。
2. 保持单行 + hover tooltip 显示全文。
3. 若要真正可变行高，放弃 DataTable，用 `v_virtual_list`（gpui-kit，`Rc<Vec<Size<Pixels>>>` 指定每项尺寸，`VirtualListScrollHandle`）或核心 `list` 自建。

### 4.4 本项目需要的其他组件（import 路径 → 关键 API）

有状态控件统一模式：构造时 `cx.new(|cx| XState::new(window, cx))` → `cx.subscribe_in(&state, window, ..)` → render 时 `X::new(&state)`。

| 需求 | 组件 | 关键 API |
|---|---|---|
| 搜索框 | `input::{Input, InputState, InputEvent}` | `InputState::new(window, cx).placeholder("Search…").clean_on_escape()`；`Input::new(&s).cleanable(true).prefix(Icon::new(IconName::Search).small()).small()`；事件 `Change / PressEnter{secondary} / Focus / Blur`；读值 `state.read(cx).value()` |
| 项目下拉（可搜索） | `select::{Select, SelectState, SelectItem, SelectEvent}` | `SelectState::new(items, Some(IndexPath::default()), window, cx).searchable(true)`；`Select::new(&s).placeholder("Project").cleanable(true).w(px(240.))`；自定义项实现 `SelectItem { type Value; title(); value(); matches(query) }` |
| 多选过滤 | `combobox::{Combobox, ComboboxState}` + `SearchableVec` | `.multiple(true).searchable(true)` |
| 时间范围 | `date_picker::{DatePicker, DatePickerState, DatePickerEvent, DateRangePreset}` | `DatePickerState::range(window, cx)`；`DatePicker::new(&s).number_of_months(2).cleanable(true)`；预设用 `DateRangePreset`；基于 `chrono::NaiveDate` |
| 搜索范围 / 展示模式（互斥） | `tab::{Tab, TabBar}` 或 `RadioGroup` | `TabBar::new("scope").segmented().selected_index(i).on_click(|ix, _, _| ..).child(Tab::new().label("User"))`。`ToggleGroup` 文档描述为多选，互斥模式未确认 |
| 左右分栏 | `resizable::{h_resizable, resizable_panel}` | `h_resizable("main").child(resizable_panel().size(px(600.)).size_range(px(300.)..px(1200.)).child(table)).child(resizable_panel().child(detail))`。文档中签名有 `h_resizable("id")` / `("id", state)` / `("id", window, cx)` 三种写法，**使用前查 docs.rs 确认** |
| 右侧详情抽屉（分栏的廉价替代） | `WindowExt::open_sheet_at` | `window.open_sheet_at(Placement::Right, cx, \|sheet, _, _\| sheet.title("Session").size(px(560.)).child(..))` |
| 消息 Markdown 渲染 | `text::{markdown, TextView}` | `TextView::markdown("id", src).max_lines(5)`（`TextViewState::is_clamped()` 判断是否折叠）；`.on_link_click(..)`；代码高亮需 `tree-sitter-*` feature |
| 搜索命中高亮 | `label::{Label, HighlightsMatch}` 或 `gpui::StyledText` | `Label::new(text).highlights("query")` 全部大小写不敏感命中；已有字节区间时 `StyledText::new(text).with_highlights(ranges)` |
| 右键菜单 / 下拉菜单 | `menu::{PopupMenu, ContextMenuExt}` | `div().id("x").context_menu(\|menu, w, cx\| menu.menu("Copy", Box::new(CopyAction)).separator())`；菜单项是 GPUI `Action`，自动显示快捷键 |
| 通知 / 对话框 | `WindowExt::push_notification / open_dialog` | `window.push_notification((NotificationType::Success, "Indexed 4,213 sessions"), cx)` |
| 滚动条 / 骨架屏 / 快捷键标签 | `scroll::*`、`Skeleton`、`Kbd` | `div().id("page").overflow_y_scrollbar()`；`Skeleton::new().w(px(250.)).h_4()`；`Kbd::new(Keystroke::parse("cmd-k")?)` |
| 侧栏 / 标题栏 | `sidebar::*`、`TitleBar` | `Sidebar::new().collapsible(true)`；`TitleBar::new().child(..)` |
| 可变高度虚拟列表 | `list::{List, ListState, ListDelegate}`、`v_virtual_list` | 详情面板消息流备选 |
| 小型静态表 | 声明式 `Table / TableRow / TableCell`（0.6 新增） | 详情面板 key/value 块；无虚拟化 |

### 4.5 推荐的应用骨架

```text
AppRoot (Entity)              状态: filters, search, display_mode, selected: Option<SessionId>
├── TitleBar
├── toolbar: h_flex — Select(project) · DatePicker(range) · Input(search) · TabBar.segmented(scope) · TabBar.segmented(display mode)
├── h_resizable("main")
│   ├── resizable_panel(60%) → DataTable::new(&self.table)
│   └── resizable_panel     → DetailPane (Entity): 元数据 + 消息列表（TextView::markdown, max_lines 折叠）
├── StatusBar: 索引进度 / session 计数 / live 指示
└── Root::render_dialog_layer / render_sheet_layer / render_notification_layer
```

数据流：规范数据存于 delegate 之外（`Arc<Vec<Session>>` 或 `Entity<SessionStore>`），delegate 只持有 `rows: Vec<usize>`（过滤+排序后的索引）与当前 `ColumnSort`。过滤事件 → 防抖 → `cx.background_spawn` 计算索引 → `table.update(.. refresh ..)`；`perform_sort` 原地排 `rows`；`TableEvent::SelectRow(ix)` → `rows[ix]` → 更新详情 entity；展示模式切换只改变 `render_td` 输出，`cx.notify()` 即可。

焦点与快捷键：`impl Focusable for AppRoot { fn focus_handle(..) { self.table.focus_handle(cx) } }` 让表格启动即可方向键导航；全局快捷键 `gpui_kit::actions!(app, [FocusSearch, ToggleMode])` + `cx.bind_keys([KeyBinding::new("cmd-f", FocusSearch, None)])` + `.on_action(cx.listener(..))`。

`DockArea`（可拖拽重排面板，Longbridge Pro 在用）存在但对本项目过重，`h_resizable` + Sheet 足够。

---

## 5. 文本、输入与键盘（核心 gpui 层）

- **`SharedString`**：`Arc<str>` 或 `&'static str`，clone 只是指针加一。行结构里存 `SharedString` 使 render 零拷贝。
- **`StyledText`**：`StyledText::new(text).with_highlights(iter<(Range<usize>, HighlightStyle)>)`；`HighlightStyle { color, font_weight, font_style, background_color, underline, strikethrough, fade_out }`；区间是 **UTF-8 字节偏移**（debug 断言在 char 边界）。Zed 的 `highlighted_label.rs` 里 `highlight_ranges(text, indices, style)` 约 30 行，把 fuzzy 命中索引合并成区间，可直接抄（它在 GPL crate 里，但逻辑平凡，自行重写）。
- **文本输入不在核心 gpui**：`examples/input.rs` 是 500 行的参考实现（`EntityInputHandler` + IME UTF-16 区间 + 自绘光标）。直接用 gpui-kit `Input`。
- **键盘**（`docs/key_dispatch.md`）：`actions!(app, [MoveUp, MoveDown])` 定义动作；`cx.bind_keys([KeyBinding::new("up", MoveUp, Some("session_table"))])`；接收方 `div().key_context("session_table").track_focus(&self.focus_handle).on_action(cx.listener(|this, _: &MoveUp, window, cx| ..))`。**没有 `key_context` + `track_focus` 的祖先，动作分发找不到目标。**原始按键 `.on_key_down(..)`。
- **字体**：macOS 通过 font-kit 取系统字体；内嵌字体 `cx.text_system().add_fonts(vec![Cow::Borrowed(include_bytes!(..))])`。
- **暗色模式**：`window.appearance()` → `WindowAppearance::{Light, Dark, VibrantLight, VibrantDark}`；`cx.observe_window_appearance(..)`。gpui-kit 已封装为 `Theme::sync_system_appearance`。

---

## 6. 异步与数据加载模式

两个执行器（Zed 博客 "Zed Decoded: Async Rust"）：**前台执行器**在主线程事件循环内，entity 只能在前台触碰；**后台执行器**是线程池，闭包必须 `Send + 'static`，不能持有 `Rc` 或 entity。`Task<R>` 被 drop 即取消，必须 `.await`、存字段或 `.detach()`。

扫描 ~900 个 JSONL 而不阻塞 UI 的惯用形态：

```rust
struct SessionList {
    rows: Vec<Rc<SessionRow>>,
    loading: bool,
    _scan: Option<Task<()>>,        // 存起来保活；drop 即取消
}

impl SessionList {
    fn start_scan(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.loading = true;
        let (tx, rx) = smol::channel::bounded::<Vec<SessionRow>>(8);
        let scan = cx.background_spawn(async move {
            for chunk in scan_sessions_in_chunks(&root, 64) {      // 阻塞 I/O + 解析，分块
                if tx.send(chunk).await.is_err() { break; }        // UI 已销毁则停
            }
        });
        self._scan = Some(cx.spawn(async move |this, cx| {
            let _keep = scan;                                       // 本任务 drop 时连带取消扫描
            while let Ok(chunk) = rx.recv().await {
                if this.update(cx, |this, cx| {
                    this.rows.extend(chunk.into_iter().map(Rc::new));
                    cx.notify();                                    // 每块 notify 一次，不要每文件一次
                }).is_err() { return; }
            }
            this.update(cx, |this, cx| { this.loading = false; cx.notify(); }).ok();
        }));
    }
}
```

要点：`cx.spawn` 给 `WeakEntity<Self>` + `&mut AsyncApp`；`this.update(cx, ..)` 返回 `anyhow::Result`，view 已销毁时失败，用 `?` 或 `.ok()`，**永远不要 unwrap**。通道用 `smol::channel` / `futures::channel::mpsc` / `postage`（都是 Zed 依赖）。几千行的排序/过滤几毫秒可以直接在主线程做；更重的把 `Arc<Vec<..>>` 快照送去 `background_spawn`。需要 tokio 的库（如某些 SQLite 异步封装）可用 `gpui_tokio::init(cx)` + `Tokio::spawn(cx, fut)` 桥接（main 分支有，0.2.2 无）。

**防抖**（Zed picker 的做法）：把 `Task` 存到字段，重新赋值即取消上一个：

```rust
self.pending_search = cx.spawn_in(window, async move |this, cx| {
    cx.background_executor().timer(Duration::from_millis(150)).await;
    this.update_in(cx, |this, window, cx| this.run_search(window, cx))?;
    Ok(())
});
```

---

## 7. 窗口、菜单、打包

```rust
fn main() {
    gpui_platform::application().run(|cx: &mut App| {     // gpui-kit: gpui_kit::application()
        cx.activate(true);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus([Menu::new("Sessions").items([MenuItem::action("Quit", Quit)])]);
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions { title: Some("Sessions".into()), ..Default::default() }),
                focus: true,
                ..Default::default()
            },
            |window, cx| cx.new(|cx| AppRoot::new(window, cx)),
        ).unwrap();
    });
}
```

- `WindowOptions`：`window_bounds`、`titlebar`、`focus`、`show`、`kind`、`is_resizable`、`window_min_size`、`window_decorations` 等；`WindowBounds::{Windowed, Maximized, Fullscreen}`。
- **记忆窗口大小**：`cx.observe_window_bounds(window, |this, window, cx| { let b = window.window_bounds(); /* 持久化 */ })`，下次启动回填 `window_bounds`。
- 菜单：`Menu { name, items }`、`MenuItem::{action, submenu, separator, os_submenu}`，见 `examples/set_menus.rs`。
- 打包：`cargo install cargo-bundle`，`[package.metadata.bundle] name / identifier / icon = ["assets/icon.icns"] / osx_minimum_system_version`，`cargo bundle --release` → `target/release/bundle/osx/*.app`。签名与公证参考 Zed 的 `script/bundle-mac`（codesign + notarytool）。

---

## 8. 坑与最佳实践清单

1. **Entity 借用规则**：`entity.update(cx, |state, cx| ..)` 期间该 entity 被租出，闭包内对同一 entity 再 `update` 会 panic（"circular entity update"）。永远用闭包传入的内层 `cx`；优先 `cx.listener` / `cx.processor` 而非手动 clone entity 进闭包。
2. **`render` 必须廉价**：不要在 `render` 里过滤/排序/格式化全量数据；预计算索引向量，只在 `uniform_list` 闭包里处理可见区间。
3. **`cx.notify()` 纪律**：不 notify 不重绘；反过来每解析一个文件 notify 一次会 60 次/秒重排。按块批量 notify。
4. **Task drop 即取消**：`cx.spawn(..)` 不 `.detach()` 也不存字段等于什么都没做。Zed 惯例字段名 `_task: Task<()>`，顺带得到取消能力。
5. **闭包里用弱引用**：entity 存的闭包若捕获 `Entity<Self>` 会形成循环引用；用 `cx.weak_entity()` / `entity.downgrade()`。
6. **`ElementId` 稳定性**：列表内交互元素要稳定 id：`div().id(("row", ix))` 或 `.id(session_id.clone())`。想让 hover/选中状态在重排序后仍跟随同一行，用 session id 而非索引。
7. **`key_context` + `track_focus`** 是动作到达 view 的前提；表格根 div 加上并在打开后 focus。
8. **`Global` 只给真正的全局单例**（主题、设置、索引句柄）；其余用 `Entity`。
9. **避免重排**：列固定宽 `w(px(..)) + flex_shrink_0() + truncate()`，文本长度就不会撑开行。
10. **`AsyncApp` 是 fallible 的**：main 分支 `AsyncApp::update` 直接返回 `R`，0.2.2 文档返回 `Result<R>`，再次说明要按锁定版本读源码。
11. **工具链漂移是最大运营风险**：Xcode 26 的 Metal Toolchain、edition 2024 + 最新 stable 才有的库特性、`gpui-pre` 每版跳一个 Zed commit。精确锁版 + 提交 Cargo.lock。

---

## 9. 相性分析：GPUI 为什么适合本项目，哪里不适合

**适合**：
- `uniform_list` 天然虚拟化，10k 行零压力；本项目几千行 session 完全在舒适区。
- Entity/notify 模型与"后台扫描 → 分块流入 → 按键重过滤"一一对应，没有 React 式 diff 开销，也没有 Electron 的 IPC。
- GPU 渲染 + Metal，滚动与重绘 60fps 是默认状态。
- 纯 Rust 单进程，数据层（SQLite / 解析 / 正则扫描）与 UI 在同一地址空间，`Arc` 共享零拷贝。
- 二进制约 12MB，启动即时。

**不适合 / 需要绕开**：
- 官方发布空窗 + 单一第三方供应商快照；API 年内还会 breaking。→ UI 层薄、核心逻辑独立 crate、精确锁版。
- 没有可变行高表格；富文本编辑弱。→ 预览列两行固定高度；消息流用 `list` / `v_virtual_list`。
- 文档稀少，很多 API 只能读源码。→ 本地保留 zed 与 gpui-kit 的 sparse clone 作为"文档"（路径见下）。
- Linux/Windows 虽支持但本项目 macOS 优先，不为跨平台妥协。

---

## 10. 参考应用与源码位置

与本项目最接近的开源 GPUI 应用（来自 zed-industries/awesome-gpui、edo-zhou/awesome-gpui，2026-09 状态）：

| 项目 | 说明 |
|---|---|
| [TokenMonitor](https://github.com/iFence/TokenMonitor) | 读取同一批 Claude Code / Codex session 数据统计 token，gpui + gpui-component |
| [Waku](https://github.com/egoist/waku) | 本地 coding agent 桌面工作台，1.4k star，有 session 列表 UI |
| [Zeron/comet](https://github.com/zeronsh/comet) | 管控 Claude Code / Codex / Cursor session |
| [zedis](https://github.com/vicanso/zedis) | Redis GUI，2.1k star，大量 key 列表与值表格 |
| [Navop](https://github.com/feigeCode/navop)、[pgui](https://github.com/duanebester/pgui)、[DBFlux](https://github.com/0xErwin1/dbflux) | 数据库客户端，大网格 |
| [rox](https://github.com/zealsprince/rox)、[hummingbird](https://github.com/hummingbird-player/hummingbird) | 上万曲目的媒体库列表 |
| [GitComet](https://github.com/Auto-Explore/GitComet)、[hunk](https://github.com/smolcars/hunk) | Git / diff 工具 |
| gpui-kit 自带 `crates/story`（70+ story）、`examples/table_in_scrollable`（129 行最小 delegate） | 官方示例 |

Zed 仓库内最值得读的文件：`crates/gpui/examples/data_table.rs`、`examples/input.rs`、`src/_ownership_and_data_flow.rs`、`docs/contexts.md`、`docs/key_dispatch.md`、`crates/picker/src/picker.rs`（uniform_list + fuzzy + 防抖）、`crates/ui/src/components/data_table.rs`、`label/highlighted_label.rs`。

本次调研的本地 sparse clone（临时目录，会被清理；需要时重新 clone）：
- `/private/tmp/claude-501/-Users-reorx-Code-scripts/d06debd1-1cb5-4f48-8652-bcbdc6190edb/scratchpad/zed/crates/{gpui,gpui_platform,ui,theme,picker}`
- `/private/tmp/claude-501/-Users-reorx-Code-scripts/d06debd1-1cb5-4f48-8652-bcbdc6190edb/scratchpad/gpui-kit`

在线：[gpui.rs](https://gpui.rs) · [docs.rs/gpui](https://docs.rs/gpui/latest/gpui/) · [gpui-kit.com](https://gpui-kit.com) · [gpui-kit v0.6.0 release notes](https://github.com/longbridge/gpui-kit/releases/tag/v0.6.0) · [crates.io gpui-pre](https://crates.io/crates/gpui-pre) · [gpui-book](https://github.com/matinaniss/gpui-book) · Longbridge 提供的 agent skill：`npx skills add longbridge/gpui-kit`。
