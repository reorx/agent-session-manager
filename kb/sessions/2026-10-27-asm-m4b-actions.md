---
created: 2026-10-27
tags:
  - m4b
  - actions
  - context-menu
  - export
  - agent-session-manager
---

# M4b 步骤 2–4：操作入口（actions.rs、读取线程的 History / 导出、⌘C / ⇧⌘C、标题行按钮、右键菜单）

## 协调者摘要（asm-m4b-actions）

- 结果：任务文件完成标准 1–5 全部满足。新模块 `actions.rs`，读取线程的 `ReadRequest` 改成枚举（含 History、Export），⌘C / ⇧⌘C，标题行按钮组，英文右键菜单，Metadata 的 Resume 行，两份文档
- 提交：代码、文档和本总结在同一个提交（`git log -1 -- kb/sessions/2026-10-27-asm-m4b-actions.md`）
- 测试：`make test` 全绿，core 166，app 118 通过、2 ignored。新增 actions 9 个（spec 的 8 个加 `check_…`）、store 3 个、键盘 3 个。键盘测试临时去掉绑定后，3 个都会失败
- 截图：`tmp/2026-10-27-m4b-actions/actions/`，6 张都用 Read 看过，脚本是同目录的 `shoot.sh`（`ASM_DB` / `ASM_STATE` / 收藏目录都在 `tmp/`）。对照画稿的结果见 §3
- 和 spec 的差异：① 恢复命令里的路径一律加单引号（spec 写的是有特殊字符才加）；id 不是纯 `[A-Za-z0-9-]` 时不给命令。② 只剩收藏时的提示用调研 §6 的措辞。③ 导出失败不走 `Failed`，改成 `Exported { dest, result }`。④ 去掉 `HideSelectedProject`，换成 `HideProject { id: None }`。细节见 §2
- 未验证：没有在实机上真点复制 / Finder / 打开目录 / 导出（不动剪贴板），这些和"resume 粘到终端执行""导出与 transcript-reader diff"都归 accept。悬停提示只看了一个置灰按钮
- 交给协调者：`product-prd.md` 第 46、101、117 行；spec PRD 第 1 节的提示措辞和引号规则要同步；`read_history` 还没接到 `select_row` 上（归 extras）。见 §5
- 关于 context：`agent-inspect-session.sh` 读不出本会话的用量（unknown）。我估计在 40% 以内，所以实机验收没开辅助 tab。中途被电脑睡眠打断过一次，恢复后从截图继续

## 1. 做了什么

- **`actions.rs`（新）**：
  - 纯函数 `available(row, original_exists, dir_exists) -> SessionActions`：每项是 `Ok` 或带英文原因的 `Err`，另带 `resume: Option<String>`
  - `check(row, &ClaudeDir)`：做两三个 stat，然后调 `available`。只有收藏的行才查原记录；项目路径必须是绝对路径且是目录
  - 另有 `resume_command`、`export_name`，以及 6 个带 `id: Option<String>` 的 action 和它们在 `AppRoot` 上的处理函数
  - 处理函数先 `check`，不能做就弹原因、不做事；能做就执行。复制成功的通知照 spec 写中文（"已复制 session ID" / "已复制 resume 命令"）
  - 导出：先弹保存对话框（默认目录是这次运行里上一次导出的目录，第一次是"下载"），然后交给读取线程写文件。`exported` 弹"已导出 X"，有坏行时加"有 N 行无法解析，已跳过"，失败时弹"导出失败：…"。对话框关闭后焦点还给表格
- **`store.rs`**：`ReadRequest::{Conversation, History, Export}`
  - 读取线程一次取完积压的请求：导出按到达顺序全部执行，`Conversation` / `History` 只读最后一个，仍按 `seq` 丢弃过期的
  - `read_history` 走 core 的 `history_prompts` → `conversation_from_history` → app 的 `build`，结果和记录文件一样回 `Conversation` 事件
  - `export` 调 `read_transcript` 再写文件
- **`main.rs`**：
  - ⌘C / ⇧⌘C 绑在 `"DataTable"` 上下文，搜索框里的 ⌘C 不受影响
  - 6 个 `on_action`，`last_export_dir`
  - `select_row` 时把 `actions::check` 的结果存进 `DetailState::actions`
  - `AppAssets` = `icon_assets!(ExtraIcons, [SquareTerminal, FileSearch, Download])` + 回落到 `Assets`
- **`detail.rs`**：
  - 标题行右侧按钮组 `render_action_buttons`，顺序是 ★、Copy resume、复制 ID、在 Finder 中显示、项目目录、导出
  - 尺寸照画稿：22px 高、14px 图标、间距 4px；置灰的按钮 `opacity(0.4)`，悬停提示写原因
  - Metadata 的 Resume 行显示 `actions.resume`，没有命令时用灰字写原因
- **`table.rs`**：`SessionTableDelegate::new(ClaudeDir)`；`context_menu` 按 spec 第 2 节重写，作用于右键点中的行
  - ⌘D / ⌘C / ⇧⌘C 三项用 `PopupMenuItem::element` 自己画快捷键提示
  - Copy Resume Command 下面一行 10px 等宽灰字，预览命令；不能复制时这一行写原因
- **`model.rs`**：删掉 `SessionRow.resume`，加上 `id_known`。`a_row_shows_…` 和 `missing_numbers_…` 两个测试里对 `resume` 的断言删掉（后者改成断言 `id_known`）
- **`toolbar.rs`**：删掉 `hide_selected_project`

## 2. 自己的决定（含与 spec 的差异）

| # | 决定 | 原因 |
|---|---|---|
| 1 | 恢复命令的路径**一律**单引号，路径里的 `'` 写成 `'\''`；spec 写的是"有空格或特殊字符时加" | 协调者的决定表写的是单引号；一律加更简单，也不会漏掉某个特殊字符。画稿里的 `~/Code/…` 不能照用：单引号里的 `~` 不展开（调研 §5） |
| 2 | `resume_command` 遇到不是纯 `[A-Za-z0-9-]` 的 id 返回 `None`，这时 Copy resume 置灰，提示 "This session has no ID" | 调研 §5：id 不能未经检查就放进 shell 命令 |
| 3 | 只剩收藏副本时的提示是 "Only the favorite copy is left; restoring from a favorite is not supported here yet"，spec PRD 表里是 "…Claude Code can no longer resume it" | 调研 §4 / §6：Claude Code 其实能按绝对路径加载副本，原措辞不对；任务文件也说措辞参考调研 §6 |
| 4 | 导出失败不走 `StoreEvent::Failed`，改成 `Exported { dest, result: Result<usize, String> }` | `Failed` 的处理会把状态栏改成 "index error"，导出失败不是索引的问题 |
| 5 | 没有 ID 的 History session：复制 ID 的提示是 "This session has no ID"；★ / Copy resume / Finder / 导出仍提示 "The transcript is gone" | spec 表里写的是"以上再加复制 ID"，所以只有新加的这一项用新原因 |
| 6 | 去掉 `HideSelectedProject`，View 菜单项改为派发 `HideProject { id: None }`，菜单文字不变 | 两个 action 做的是同一件事，留一个 |
| 7 | Metadata 的 Resume 行没有命令时用灰字写原因，不写 "—" | 不加按钮，就能看出为什么不能 resume |
| 8 | 右键菜单的 Favorite 项也画 ⌘D 提示；Hide Project 用弯引号 `“…”` | 画稿 6 用的是弯引号；收藏有快捷键，提示应一致 |
| 9 | 右键菜单的项高用组件默认的 26px（画稿是 22px），悬停色用主题色 | 和工具栏的下拉菜单保持一致；画稿里是蓝底白字 |
| 10 | 打开项目目录的图标用 `folder-open`（画稿画的是闭合的文件夹） | spec 现状一节列的就是 `folder-open`，意思也是"打开" |

spec 风险第 3 条的结论：`PopupMenuItem::element` 支持 `disabled`，点击后照常派发 action 并关闭菜单，**不需要**退回 `menu_with_disabled`。另外踩到一个坑：element 项的外层没有 `min_w_0`，长的命令预览会把菜单撑宽，`truncate` 不起作用。办法是给内容固定宽度（`MENU_ITEM_W = 330 − 24`）。已写进 `gpui-kit-guide.md`。

## 3. 截图与画稿对照

目录 `tmp/2026-10-27-m4b-actions/actions/`，每张都用 Read 看过：

| 截图 | 画面 | 和画稿的差异 |
|---|---|---|
| `1-normal-header.png` | 普通 session（`4add415e`，项目目录在），6 个按钮都可用 | 画稿 1 没有 ★（plan 之后定的），排在最前；顺序、尺寸、边框都一致 |
| `2-history-header.png` | History session（`46b8c9a8`，accounting）：★、Copy resume、Finder、导出置灰；复制 ID、项目目录可用 | 和画稿 5 的置灰集合一致（★ 是新增的，也置灰）。Conversation 区还是 M4a 的说明句，History 卡片归 extras |
| `3-favorite-original-gone.png` | 合成数据：fixture S1 的 cwd 改成本仓库，收藏后删掉原记录。★ 是实心的，只有 Copy resume 置灰 | 没有对应画稿；行为符合 PRD 第 1 节第 3 行 |
| `4-context-menu.png` | 右键点 `dotfiles` 行（选中仍在 `4add415e`），预览是 `cd '/Users/reorx/Code/dotfiles' && claude --resum…` | 和画稿 6 的顺序一致，最前面多了 Favorite ⌘D；项高和悬停色见 §2 #9 |
| `6-history-context-menu.png` | 右键点 History 行：Favorite、Copy Resume、Reveal、Export 置灰，预览行写 "The transcript is gone" | — |
| `5-hover-1271-89.png` | 悬停在置灰的 Copy resume 上，提示 "The transcript is gone" | — |

第一版的菜单里命令预览没有截断，⇧⌘C 被推到菜单外面。修好以后重截了 4 号图。

## 4. 接口清单（给 asm-m4b-extras）

- **History 请求**：`Store::read_history(session_id, project_key, matcher) -> u64`（`store.rs`），回的是普通的 `StoreEvent::Conversation`，和 `read_conversation` 共用 `seq` 并互相取代。现在标着 `#[allow(dead_code)]`，`select_row` / `open_conversation` 的 `transcript_path.is_none()` 分支接上它以后删掉这个标记
- **History 标志**：`store.rs::read` 已经拿到 core 的 `Conversation`（`history_only` 为 true），要把这个标志传进 `ConversationView` 时改 `view_of` / `conversation::build`
- **History 的说明句**：旧的一句在 `turn_cards.rs` 第 138 行附近（`row.transcript_path.is_none()`），由 extras 换成顶栏下面的灰字
- **action 列表**（`actions.rs`）：`CopySessionId`、`CopyResume`、`RevealTranscript`、`OpenProjectFolder`、`ExportTranscript`、`HideProject`，都带 `{ id: Option<String> }`；收藏仍是 `favorites::FavoriteSession { id }` / `ToggleFavorite`
- **按回合跳转**：`PrevTurn` / `NextTurn` 还没建，`[` / `]` 加到 `main.rs::bind_keys` 的 `"DataTable"` 上下文
- **标题行组件**：`detail.rs::render_detail_header` 调 `render_action_buttons(row, cx)`，按钮外观是 `action_button` + `tip`；`DetailState::actions` 在 `select_row` 里设置。没有搜索时的 `[ ]` 提示放在对话顶栏，不在这里
- **Metadata**：`detail.rs::render_metadata`，Files 一项接在 Top tools 后面；Resume 行是 `resume_value`
- **`SessionRow.id_known`**：新加的字段，`SessionRow.resume` 已经删了

## 5. 交给协调者

- `kb/docs/product-prd.md`（归 accept）：
  - 第 46 行详情面板的"⬜ 操作入口"
  - 第 101 行"待补充：右键菜单…"，现在已实现
  - 第 117 行隐藏项目：多了一个右键菜单入口
- spec PRD 第 1 节：
  - 收藏提示的措辞（§2 #3）
  - 恢复命令一律加引号（§2 #1）
  - "项目目录不存在"一行的行为按协调者的表（`claude --resume <id>`，Copy resume 可用）
- 已知问题：这次没有发现新的需要登记的问题
