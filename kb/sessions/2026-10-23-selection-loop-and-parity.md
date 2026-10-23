---
created: 2026-10-23
tags:
  - bugfix
  - selection
  - parity
  - herdr
  - agent-session-manager
---

# 修复选中事件死循环，补齐 parity 里的空项目目录

## 概要

费用列验收时发现 autoplay 第二步会让主线程空转（改动前的版本同样复现）。用户让我判断还剩哪些问题，并用 Herdr 开 Astra（Codex）去解决。我把「选中事件死循环」和「parity 一直红」写成任务简报（`kb/plans/2026-10-23-selection-loop-and-parity.md`），交给 Astra 在同一个 checkout 里做，约束它不提交；它交回报告后我核对代码、测试和截图，更新文档并统一提交。

- **死循环的根因**：`TableState::set_selected_row` 先改好表格的选中行，再排一个 `SelectRow` 事件，不论是否重复。一次 update 里选中两次（autoplay 第二步里清项目、改范围、选项目各触发一次 `after_rows_changed`，最后又显式选第 0 行）就排了多个事件；处理旧事件时把选中拉回旧行、又发一个事件，两个事件来回触发，`state.json` 被反复重写。修复：`on_table_event` 只处理与 `selected_row()` 一致的 `SelectRow`。真实点击和方向键都是先改表格再发事件，所以不会被丢掉。修复前写的复现测试 0.05 秒内失败（报 34 次事件），没有把测试进程挂死。
- **parity**：`asm list` 现在也列出只有 `memory/` 或 `sessions-index.json` 的项目目录（0 sessions），`list-tenderbuddy` 转绿。剩下的 `list-code-30d` 是空 session 并列时的顺序：Python 继承 `glob` 的目录遍历顺序，没有次关键字。简报规定这种情况不改 Python、只报告，所以仍有 1 组失败。

## 修改的文件

- `crates/app/src/main.rs`：`on_table_event` 只处理当前选中行的 `SelectRow`；测试专用的事件计数熔断（只在 `cfg(test)` 下存在）
- `crates/app/src/keyboard_tests.rs`：4 个测试，分别覆盖同一 update 连续选中、连续点击式通知后方向键仍可用、切到空项目后旧通知不复活选中、autoplay 第二步的组合操作
- `crates/core/src/bin/asm.rs`：`resolve_project_keys` 并入 `projects/` 下的目录名
- `crates/core/tests/cli.rs`：只有 memory / 元数据的项目目录会被列出，普通文件不会
- `kb/plans/2026-10-23-selection-loop-and-parity.md`（新）：派给 Astra 的任务简报
- `kb/known-issues.md`：删掉 autoplay 卡死；parity 条目改为只剩并列顺序并写明原因；新增「点表头排序后选中高亮和详情对不上」
- `kb/docs/app-architecture.md`「选中」一节、`AGENTS.md` 的 GPUI 规则：写明 `SelectRow` 是通知，旧事件要丢弃

## 注意事项

- GPUI 的事件在当前 update 结束后才送达。把组件发出的状态通知当成命令再执行一遍，就可能和组件自己的状态互相触发；处理前先和组件的当前状态核对。
- 会挂死 effect 队列的 bug，测试里要加一个能自己停下来的熔断，再在队列排空后断言，否则 `run_until_parked` 永远不返回。
- 派出的 agent 在同一个 checkout 里工作、不提交，由派出方统一提交，这样满足「代码和文档同一个提交」的要求。Astra 约 8 分钟完成，报告在 `tmp/2026-10-23-followups/report.md`，截图在同目录。
- 验证：`cargo test --workspace` 全过（app 73，core 105），clippy 没有新增警告；`make screenshot` 三张图都正确，第 3 张是「claude-cwd · 30 days · Cost 降序 · Metadata」；`make parity` 两遍分别是 19/21（另一组是 search 偶发）和 20/21。

## 遗留问题

- parity 的并列顺序：要统一，需要改用户自己的 Python 参考脚本，等用户决定（已登记 known-issues）。
- 点表头排序后，选中高亮停在原来的行号上，详情还是原来的 session。这是从源码推断的，未实机验证（已登记 known-issues）。
- 可选的「冷启动首批行上屏」新旧对比 Astra 没做，费用列对冷启动首屏的影响仍没有数字。
- `make screenshot` 只固定等 2.5 秒就找窗口，新编译的二进制首次启动可能来不及，Astra 第一次运行就因此失败、重跑才成功。

## 已解决的已知问题

- **截图：autoplay 第二步在真实数据上让主线程空转，`make screenshot` 的第 3 张停在第一步**：`on_table_event` 丢弃过期的 `SelectRow` 通知；修复前写的复现测试失败、修复后通过，`make screenshot` 第 3 张显示排序后的视图。

## 相关文档

- [选中事件死循环与 parity 任务简报](../plans/2026-10-23-selection-loop-and-parity.md) — 本次新建，派给 Astra 执行
- [费用 / context 列 session 总结](2026-10-23-cost-context-columns.md) — 发现死循环的那次验收
- [运行时架构](../docs/app-architecture.md) — 「选中」一节补充事件规则
