---
created: 2026-10-23
tags:
  - bugfix
  - selection
  - parity
  - herdr
  - agent-session-manager
---

# 表头排序后选中跟着 session 走；parity 恢复全绿

## 概要

用户对上一轮遗留问题的答复：统一 parity 的并列排序（同意改参考脚本），修表头排序后的选中错位，M4 先跳过。我自己做 parity，Astra（`asm-fix`）做排序选中，两边改的文件不重叠。

- **parity**：Python 参考脚本按目录遍历顺序收集 transcript，再只按开始时间做稳定排序，没有开始时间的 session 并列时顺序取决于文件系统。给脚本加了 `newest_first`：开始时间降序，并列时更新时间降序，再按 session id，`--list` 和 `--session` 都用它。Rust 的 `ORDER BY` 本来就是这个规则，不用改，补了一条 CLI 测试把它固定住。`make parity` 两遍都是 21/21。脚本仓库的提交是 `~/Code/scripts` `10e1a6b`。
- **表头排序选中错位**：Astra 先在真实 app 里用 cliclick 点表头确认了问题：高亮停在原来的行号上，那一行已经换成别的 session，详情还是原来那个。根因是 gpui-component 的 `perform_sort` 只重排行，不改 `selected_row`，而 `AppRoot` 对表格的 observe 只保存视图状态。修复：observe 记下上一次的排序，一变就走 `after_rows_changed`，按 session id 重新定位选中；记录先更新，重新定位时再通知表格也不会自我循环。修复前写的测试失败，修复后通过。实机连续点 Cost 三次（降、升、默认），高亮和详情始终是同一个 session。

## 修改的文件

- `~/Code/scripts/claude_session_search.py`、`test_claude_session_search.py`（新）、`AGENTS.md`：并列排序规则和测试，另一个仓库，已单独提交
- `crates/core/tests/cli.rs`：没有 prompt 的 session 按更新时间降序、再按 id 排
- `crates/app/src/main.rs`：表格 observe 里检测排序变化，重新定位选中
- `crates/app/src/keyboard_tests.rs`（Astra）：普通列三态排序、空表排序、搜索时按 Matches 排序，三个场景
- `kb/plans/2026-10-23-sort-keeps-selection.md`（新）：派给 Astra 的简报
- `kb/known-issues.md`：删掉 parity 和表头排序两条
- `kb/docs/app-architecture.md`「选中」：排序也属于「选中跟着 session 走」，并说明 observe 的做法
- `AGENTS.md`：`make parity` 一行写明两边共用的排序规则和偶发的 search 失败

## 注意事项

- 组件内部改了自己的状态却不发事件的地方（这里是表头排序），应用只能靠 observe 发现；在 observe 里比较前后状态再动作，并且先更新记录，否则自己发出的通知会让回调再跑一遍。
- 表头点击在无界面测试里点不到：排序图标没有为测试登记 id，按坐标派发点击也没触发排序。测试改走 `TableDelegate::perform_sort`，也就是组件私有的 `TableState::perform_sort` 里调用的同一入口，图标命中和箭头状态由实机 cliclick 验证。
- Astra 的 context 过了 40%，按 CLAUDE.md 自己开了验收 tab `w4E:t4`（`sort-verify`）。报告和截图在 `tmp/2026-10-23-sort-selection/`。
- 验证：`cargo test --workspace` 全过（app 76，core 106），clippy 没有新增警告，`make parity` 两遍 21/21。

## 遗留问题

- 费用相关的两条 known-issues 仍在：自算费用偏低，以及新模型的调用不计价、表格不提示。
- 冷启动首批行上屏的新旧对比仍没有数字。
- `make screenshot` 只固定等 2.5 秒就找窗口，新编译的二进制首次启动可能来不及。
- M4 详情面板按用户意见暂缓。

## 已解决的已知问题

- **回归验证：parity 因空 session 的并列顺序一直有 1 组失败**：参考脚本补上「更新时间降序、再按 id」的次关键字，与 Rust 一致，两边各有测试；`make parity` 两遍 21/21。
- **表格：点表头排序后，选中高亮停在原来的行号上，和详情面板对不上**：表格 observe 检测排序变化后按 session id 重新定位选中；修复前写的测试失败、修复后通过，实机连续三次排序都对。

## 相关文档

- [表头排序保持选中任务简报](../plans/2026-10-23-sort-keeps-selection.md) — 本次新建，派给 Astra
- [选中事件死循环与 parity 简报](../plans/2026-10-23-selection-loop-and-parity.md) — 上一轮，parity 的原因在它的报告里查明
- [运行时架构](../docs/app-architecture.md) — 更新「选中」一节
