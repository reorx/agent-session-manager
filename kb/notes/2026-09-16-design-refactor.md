---
created: 2026-09-16
tags:
  - gpui
  - gpui-kit
  - ui
  - design
  - agent-session-manager
---

# 界面按设计稿重构（2026-09-16）

设计稿：Claude Design 项目 `Agent Session Manager.dc.html`（https://claude.ai/design/p/b06a59e0-d124-4457-885f-9aef3c9687ff）。1440×900，macOS 原生外观 + 数据应用密度：12px 正文、11px 次级、36px 两行表格行、22px 控件；顶部统一为「标签 值 ▾」下拉；右侧 580px 详情面板带 Metadata / Conversation 两个 tab；Conversation 里工具调用与 thinking 默认收成 22px 单行。

## 代码结构（`crates/app/src`）

| 文件 | 内容 |
|---|---|
| `main.rs` | `AppRoot`：状态、事件、快捷键、autoplay、Preview 列宽同步、窗口与菜单 |
| `style.rs` | 设计稿全部色值（按名字取，如 `style::muted()`）、Kind 徽章、`install_theme()` 把配色写进 gpui-kit 主题 |
| `model.rs` | `SessionRow` / `Kind` / `DisplayMode` / `ScopeSet` / `RangePreset` / `Filter` / `Message` |
| `fake.rs` | 10k 假行、项目计数、按行拼出的假对话（M1 接真实数据后整个文件删掉） |
| `table.rs` | `SessionTableDelegate`：filter + sort 产出 `ix`，`render_td` 只取预格式化字符串 |
| `toolbar.rs` | 四个下拉（`Button::dropdown_menu` + `PopupMenuItem::element`）+ 搜索框 |
| `detail.rs` | 详情面板：头部、Metadata 网格、Conversation（chip 过滤、折叠 / 展开、Expand all） |

## gpui-kit 用法与坑

- **全局密度**：`Theme::global_mut(cx).font_size = px(12.)`。`Root` 每帧 `window.set_rem_size(theme.font_size)`，所以 rem 变 12 后组件内部的 `text_base` / `h_6` 等全部按比例缩小，正好落到设计稿的密度；需要精确尺寸的地方用 `px()` 显式写
- **改主题色**：改 `theme.colors.*` 后必须 `theme.tokens = ThemeTokens::from(&theme.colors)`（DataTable / StatusBar 读的是 `tokens`），最后 `Theme::sync_base(cx)` 同步到 gpui-base（滚动条等）
- **选中行是覆盖层**：DataTable 把 `table_active` 画成一个 absolute 的 div 盖在单元格**上面**（`state.rs` "Row selected style"），所以选中色必须半透明（`accent().opacity(0.16)` ≈ 设计稿的 `#dde8fa`），不透明色会把整行内容盖住；`table_active_border` 设成透明去掉 1px 描边
- **下拉菜单**：`Button` 实现了 `DropdownMenu`，`.dropdown_menu(|menu, w, cx| ..)`；builder 在每次打开时重建（dismiss 后 state 清空），所以直接捕获当前选中值来画 ✓ 即可。菜单项用 `PopupMenuItem::element(..).checked(bool).on_click(..)`，回调里通过 `WeakEntity<AppRoot>::update(cx, ..)` 改状态。右对齐用 `dropdown_menu_with_anchor(Anchor::TopRight, ..)`
- **Button 自定义外观**：`ButtonCustomVariant::new(cx).color().hover()` 定底色，边框 / 高度 / 圆角直接在 Button 上链 `Styled`（实例样式最后 refine，能覆盖变体）；`.child()` 放任意内容，`.dropdown_caret(true)` 加箭头
- **Preview 列自适应**：`Column` 没有 flex，`DataTable` 横向虚拟化。做法：`h_resizable(..).with_state(&Entity<ResizableState>)`，每帧在 `AppRoot::render` 里读 `sizes()[0]` 算出 Preview 宽度，变化超过 0.5px 时 `delegate_mut().set_preview_width()` + `table.refresh(cx)`（首帧 sizes 为空，用 viewport - 580 兜底）
- **`impl IntoElement` 返回值**：edition 2024 下 `fn f(cx: &App) -> impl IntoElement` 会捕获 `cx` 的生命周期，连续调用两次就借用冲突；返回 `Div` / `AnyElement`，或写 `-> impl IntoElement + use<>`
- `Window::viewport_size()` 无参；`InputState::focus_handle(cx)`（`Focusable` trait）+ `FocusHandle::focus(window, cx)`
- `cx.theme()` 需要 `use gpui_kit::component::ActiveTheme`；`overflow_y_scrollbar()` 在 `component::scroll::ScrollableElement`

## 与设计稿的已知差异

- 表头 36px 而非 24px：DataTable 表头行高与 `table_row_height` 绑定，不可单独设
- 所有可排序列都显示 ⇅ 图标（`render_sort_icon` 私有），设计稿只在当前排序列显示箭头
- Project 下拉没有 "Filter projects…" 输入框（假数据只有 12 个项目；接真实数据后项目会到 85 个，届时改用 `Select::searchable`）
- Updated 下拉没有 "Custom…"（日期区间选择器留到过滤真正接上数据时再做）
- Kind 列右侧留了约 16px 给纵向滚动条 + DataTable 的尾部空列

## 验收

`make screenshot`（autoplay：滚到 5000 行 → 过滤 scripts / 30 天 + 按 Prompts 降序 + Metadata tab）和 `make screenshot-clicks`（cliclick 真点击：四个下拉、选项目、切 Preview、选行、展开 thinking / tool、关 Tools chip、切 Metadata）。截图归档 `tmp/2026-09-16-design-refactor/`（12 张）。
