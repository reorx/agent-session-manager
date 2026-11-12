---
created: 2026-11-12
tags:
  - table
  - cache
  - detail-panel
  - conversation
  - agent-session-manager
  - plan
---

# 列显示开关、精确命中率、回合卡标出缓存失效

## PRD

### 背景

2026-11-12 表格加了 Hit / Hit% 两列（见 `2026-11-10-cost-analysis.md`「后续」），带来三个问题：

1. 列多了，默认 1440 宽、详情打开时表格放不下，Kind 要横向滚动（`known-issues.md`）。
2. Hit% 是整数，本机 421 个 session 都是 `99%`，但 99.0% 和 99.8% 的未命中量差 5 倍。
3. 看到某个 session 命中率低，不知道缓存是在哪一轮、为什么失效的。

本机最近 30 天 72,719 次主线调用的实测（`tmp/cache-breaks.py`）：70,903 次比上一次调用少读的缓存不超过 1k token，失效的 455 次都超过 10k，二者分得很开。原因：空闲超过缓存时长 282 次（重写 49.2M token），compact 之后 145 次（6.5M），换模型 5 次（0.6M），不明 23 次（2.8M，间隔只有 2–17 秒，多数不明的发生在同一时段）。

### 功能与交互

- **列显示开关**：在表头任一列上右键，弹出菜单列出所有可隐藏的列（Project、Started、Updated、Prompts、Cost、Ctx、In、Hit、Hit%、Out、Size、Kind），勾选表示显示；点一项切换。最后一项「Show All Columns」。Preview 和搜索时的 Matches 不在菜单里，始终显示。隐藏的列重启后仍然隐藏。隐藏了正在排序的列，排序回到默认（Updated 降序）。Preview 列随之变宽。默认全部显示。
- **精确命中率**：详情 Metadata 的 Tokens 一行末尾加 `· hit 99.3%`（一位小数，向下取整；没有输入不显示）。
- **回合卡标出缓存失效**：Conversation 里，发生缓存失效的回合在状态行下方多一行琥珀色说明，例如：
  - `cache rebuilt · 135k written · $1.08 · idle 2h 10m, cache lasts 1h`
  - `cache rebuilt · 22k written · $0.11 · after compaction`
  - `cache rebuilt · 80k written · $0.40 · model changed from Opus 4.7`
  - `cache rebuilt · 103k written · $0.82 · cause unknown`
  一个回合有多次失效就列多行。Conversation 顶部的摘要（`9 turns · 214 tool calls`）在有失效时加上 `· 3 cache rebuilds`。

### 不做的事

- 不改列的顺序、不支持拖动排序列。
- 不在表格里加「失效次数」列，不把失效统计存进索引（只在读 Conversation 时现算）。
- 不看 subagent 的缓存（它们的调用在各自的文件里，缓存独立）。
- 不判断「不明」原因的细节（CLAUDE.md 变化、工具列表变化等），记录里看不出来。

### 验收标准

- 表头右键能隐藏 / 显示列，隐藏 Started、Size、Kind 后详情打开时不再横向滚动；重启后保持；隐藏正在排序的列后排序回到默认。
- Metadata 的 Tokens 行显示一位小数的命中率。
- 在一个有 compact 和长时间空闲的真实 session 上，对应回合出现说明行，原因正确；摘要里的次数与 `tmp/cache-breaks.py` 对这个 session 的结果一致。

## Tech Design

### 现状

- 表格列在 `table.rs::SessionTableDelegate.columns`，`new()` 里一次建好；Matches 列在搜索时插到 `MATCHES_COL = 2`；`fixed_cols_width()` 用常量 `FIXED_COLS_W` 算 Preview 的宽度；`set_preview_width` 按 `PREVIEW_COL` 改列宽。
- gpui-component 的表格只给行右键菜单（`TableDelegate::context_menu(row_ix, …)`，挂在整个表格上，`right_clicked_row` 为 `None` 时菜单为空，空菜单不弹出）。`render_th` 由 delegate 渲染表头格子的内容。
- Conversation 由 core `claude/conversation.rs::Reader` 读出，`transcript::Message` 只解析 `id / stop_reason / content`，不看 `usage` 和 `model`。

### 方案

**列显示开关**

- `table.rs`：`all_columns`（全部列定义）+ `hidden: BTreeSet<SharedString>`，`columns` 由二者（加上搜索时的 Matches）重建；`PREVIEW_COL` / `MATCHES_COL` 改为按 key 查位置；`fixed_cols_width()` 改为对可见列求和（Preview 除外），删掉 `FIXED_COLS_W`。`set_hidden_columns(keys)`、`toggle_column(key)`、`hidden_columns()`。隐藏正在排序的列时 `sort = None`。
- 表头右键：`render_th` 里给格子内容挂 `context_menu`，菜单项用 `PopupMenuItem::new(name).checked(visible).on_click(...)`；右键按下时先 `set_right_clicked_row(None)`，避免表格自己的行菜单拿到上次右键的行。点击后通过 `cx.emit` 不可行（delegate 没有自己的事件），改为派发 action `ToggleColumn { key }` / `ShowAllColumns`，`AppRoot` 处理：改 delegate、`refresh`、`sync_preview_width`、`persist`。
- `settings.rs::ViewState.hidden_columns: Vec<String>`。

**精确命中率**

- `model.rs::tokens_summary` 加 `· hit 99.3%`（整数运算：`read * 1000 / total`，一位小数）。

**缓存失效**

- core `transcript::Message` 加 `model`、`usage`（复用 `parse.rs::RawUsage`）。
- core `model.rs`：`CacheBreak { ts, dropped, written, cost_usd: Option<f64>, cause: CacheBreakCause }`，`CacheBreakCause::{Idle { gap_ms, ttl_ms }, Compaction, ModelSwitch { from }, Unknown}`；`Turn.cache_breaks: Vec<CacheBreak>`。
- `Reader`：按 `message.id` 去重的每次主线调用（跳过 `<synthetic>`），记上一次调用的 prompt（未缓存 + 缓存写 + 缓存读）、模型、时间、缓存时长（最近一次有写入的调用写的是 1h 还是 5m）、两次调用之间是否有 compact。`dropped = prev_prompt − cache_read`，超过 `CACHE_BREAK_MIN = 10_000` 记一次失效，挂在当前回合上。原因优先级：compact → 换模型 → 间隔超过缓存时长 → 不明。费用按该调用的缓存写 token 用 `pricing::price_tokens` 算（含 fast、长 prompt）。
- app `conversation.rs`：`TurnCard.cache_notes: Vec<SharedString>`；`summary` 加次数。`turn_cards.rs::render_status` 下方渲染说明行（`style::cache_note()` 琥珀色）。

### 实施步骤

- [x] 1. Metadata 命中率：测试 → 实现
- [x] 2. 列显示开关：table 测试（隐藏 / 显示、宽度、排序复位、Matches 位置）→ 实现 → 表头菜单与 action → ViewState
- [x] 3. core 缓存失效：测试（`tests/conversation.rs` 用内联记录，没有改 fixture 生成脚本）→ 实现
- [x] 4. app 回合卡说明行与摘要：测试 → 实现
- [x] 5. 验收截图：列菜单（cliclick 右键表头）、隐藏列后的布局、回合卡说明行（`screenshot-cost` 加了 4 步，`ASM_AUTOPLAY_CACHE_SESSION` 指定 session）
- [x] 6. 文档（AGENTS.md、product-prd、known-issues 删掉宽度那条）、commit

### 风险与待定

- 表头格子内容上的 `context_menu` 与表格整体的行菜单都在冒泡阶段响应右键；行菜单在右键时读 `right_clicked_row`，若残留上次的行会多弹一个菜单。先清空再试，不行就改为在 View 菜单里放列开关。
- 原因判断依赖记录里的时间戳；resume 进来的 session 第一条调用与上一次运行间隔很长，会归为空闲，这是对的。
- 失效原因的官方依据：Claude Code 列出的会 / 不会让缓存失效的操作、主对话与其他请求的 TTL 分桶见 `kb/refs/claude-code-prompt-caching.md`；缓存寿命从请求开始时算、读命中会续期见 `kb/refs/claude-api-prompt-caching.md`。

### 实现与计划的差异

- `update_columns` 不带 `Window`：render 每帧都会按面板宽度重算 Preview，不需要在 action 里同步。
- 菜单项用 action（`ToggleColumn { key }` / `ShowAllColumns`，定义在 `table.rs`），不用 `on_click` 闭包：要在 `AppRoot` 里存视图状态。
- 一次 API 响应会写成多条记录、每条带同样的 usage，按 `message.id` 去重，否则第二条会把第一条当成「上一次调用」，误报失效（测试 `a_response_written_as_two_records_counts_once…`）。
- 缓存时长取最近一次有写入的调用：写了 1h 的就按 1 小时算，否则 5 分钟。
- 用 fast mode 的切换核对过「不明」的 23 次：本机没有一次是 fast / 标准切换造成的，所以不单列这个原因。不明的那些调用读回的都约为 29.6k（正好是系统提示加工具定义），推测是 Claude Code 改写了更早的消息，记录里看不出来。
- core fixture 里的 S1 最后一次调用隔了一天，读回的缓存少了 26k，现在被识别为一次失效，`store.rs` 的测试期望随之改为 `4 turns · 3 tool calls · 1 cache rebuild`。

### 验收（2026-11-12）

截图在 `tmp/2026-11-12-columns-cache-breaks/`，`ASM_AUTOPLAY_CACHE_SESSION=eca9490a make screenshot-cost SHOT_DIR=…` 与 `tmp/header-menu-shot.sh`（cliclick 右键 Cost 表头、再点菜单项）。

- 列开关：`9-hidden-columns.png` 隐藏 Started / Size / Kind 后详情打开也不再横向滚动；`state.json` 存下 `["kind", "size", "started"]`。用这份状态重启（`h1-restarted.png`）仍然隐藏；右键表头弹出菜单（`h2-menu.png`），三项未勾选，右键没有触发排序、没有弹出行菜单；点「Kind」后 Kind 列回来、选中行不变，状态变为 `["size", "started"]`（`h3-after-click.png`）。
- Metadata：`8-metadata-hit.png` Tokens 行 `… write 843k · hit 96.9%`，表格同一行 Hit% 为 `96%`。
- 缓存失效：`eca9490a` 摘要 `10 turns · 15 auto · 180 tool calls · 4 cache rebuilds`，与 `tmp/cache-breaks-per-session.py eca9490a` 的 4 次一致（3 次空闲、1 次 compact）；TURN 2 的说明 `cache rebuilt · 175k written · $1.40 · idle 2h 10m, cache lasts 1h`（`7-cache-note.png`）与脚本的 175,199 token、07:36 UTC 对得上。
- `make test` 全过，`make parity` 21 组全过。
- 默认仍显示全部列，所以第一次在 1440 宽、详情打开时还是会横向滚动，要用户自己隐藏几列；known-issues 里那条已删。

