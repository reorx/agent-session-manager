---
created: 2026-10-23
tags:
  - bugfix
  - selection
  - table
  - agent-session-manager
  - brief
---

# 任务简报：点表头排序后，选中要跟着 session 走

- 执行者：Herdr tab `w4E:t3` 里的 Astra（`asm-fix`，上一个任务也是你做的）。由派出你的 Claude agent 撰写；用户已同意修这个问题，具体行为是派出方定的，你有不同意见可以在报告里写
- 目录：`/Users/reorx/Code/agent-session-manager`，`master`，起点 commit `11aae2c`。同一个 checkout，不建 worktree
- 开工前读：`kb/known-issues.md` 里「表格：点表头排序后……」一条；`kb/docs/app-architecture.md`「选中」一节（含你上次补的 `SelectRow` 规则）；`kb/docs/gpui-kit-guide.md`「表格 → 排序状态 / 选中行」「测试」「脚本化交互与截图」；实机前读 `~/.claude/docs/DESKTOP_APP_TESTING.md`

## 背景

gpui-component 的表头点击走 `TableState::perform_sort`（`~/.cargo/registry/src/index.crates.io-*/gpui-component-0.6.1/src/table/state.rs:1150`），它调 `SessionTableDelegate::perform_sort`（`crates/app/src/table.rs`）重排 `ix`，但不改 `selected_row`。`AppRoot` 对表格的 `observe` 只调 `persist`。所以点表头后高亮停在原来的行号上，那一行已经是另一个 session，详情面板仍是原来的 session。这是读源码得出的结论，**还没有实机确认**，第一步先确认。

## 目标

点表头排序（升、降、回到默认三种状态都算）之后，原来选中的 session 仍然被选中、高亮在它的新位置、详情不变；`selected_id` 照常保存。这与过滤变化时的规则一致（「选中」第 1 条）。

## 完成标准

1. 实机确认问题存在（或不存在）：真实 app 里选中一行，用 cliclick 点一个表头，截图前后对比高亮行和详情标题。不存在就停下来报告
2. `keyboard_tests.rs` 里有复现测试：修前失败、修后通过。测试应尽量走和表头点击相同的入口（`perform_sort` 是私有的；可以试试在测试窗口里向表头位置派发鼠标点击，或别的办法），做不到就说明你用了什么替代入口、它和真实点击差在哪
3. 修复后实机再点一次表头，截图里高亮行与详情标题是同一个 session；连续点三次（降、升、默认）都对
4. 表格为空、或选中的 session 排序后仍在表格里这两种情况都不出错；不能引入新的选中事件循环（你上次的熔断计数器可以复用）
5. `cargo test --workspace` 全过，clippy 不新增警告
6. 报告写在 `tmp/2026-10-23-sort-selection/report.md`，截图归档在同目录

## 已经定下的事

| 问题 | 决定 | 谁定的 |
|---|---|---|
| 排序后选中谁 | 原来选中的 session，按 session id 找到新位置并选中。`set_selected_row` 会顺带滚动到那一行，接受这个滚动，和过滤变化时的表现一致 | 派出方（依据「选中跟着 session 走」的既有规则和 known-issues 的处理方向） |
| 不选第一行 | 不做「排序后滚到顶、选第一行」，那会让详情跳到别的 session | 派出方 |
| 搜索中的 Matches 列排序 | 同样适用 | 派出方 |

## 怎么做

- 测试先行。实现方式你定，例如在 `observe` 回调里比较 `sort_state()` 和上次记下的值，变了才走 `after_rows_changed`；注意 `after_rows_changed` 自己会 notify 表格，回调必须能停下来
- 所有起 app 的命令把 `ASM_DB`、`ASM_STATE` 指到 `tmp/2026-10-23-sort-selection/`（可从 `tmp/2026-10-23-followups/index.sqlite` 复制索引）；不要碰真实索引
- 交互只用 cliclick 鼠标点击、预设 `state.json`、autoplay；**不要**用 System Events / osascript / cliclick 发按键，不要动剪贴板。脚本启动的窗口先点一下标题栏激活，点击前先悬停（见 `scripts/screenshot-clicks.sh` 的 `click()`）
- 截图以打开图片看到的为准；app 用完 kill，`pgrep -fl agent-session-manager` 为空，最后 `mac-dev-cleanup --min-age 2`

## 约束

- 可以改：`crates/app/src/` 下的代码与测试；如有必要可以在 `scripts/screenshot-clicks.sh` 末尾加表头点击的步骤
- 不要改：`AGENTS.md`、`kb/` 下任何文件、`~/Code/scripts/`（派出方正在改那个仓库里的 Python 脚本，避免冲突）、core crate
- 不要 commit、不要 push；改动留在工作区，派出方核对后连同文档一起提交

## 交付

`tmp/2026-10-23-sort-selection/report.md`：实机确认的结果（前后截图路径）；根因；修法；测试走的入口；三次点击的截图结论；改了哪些文件；测试与 clippy 结果；没处理的问题和不同意见。
