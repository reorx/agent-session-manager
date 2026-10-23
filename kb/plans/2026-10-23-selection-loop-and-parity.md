---
created: 2026-10-23
tags:
  - bugfix
  - selection
  - parity
  - agent-session-manager
  - brief
---

# 任务简报：选中行事件死循环、parity 恢复全绿

- 执行者：Herdr 新 tab 里的 Astra（Codex），由开发费用列的 Claude agent 派出；本简报由它撰写，其中的取舍它做主、用户尚未逐条确认，你有不同意见可以在报告里写
- 目录：`/Users/reorx/Code/agent-session-manager`，分支 `master`，起点 commit `50dd8cc`。直接在当前 checkout 工作，不建 worktree、不切分支
- 开工前读：`AGENTS.md`；`kb/known-issues.md` 里「截图：autoplay 第二步……」和「回归验证：parity……」两条；`kb/docs/app-architecture.md` 的「选中」「键盘焦点」两节；`kb/docs/gpui-kit-guide.md` 的「表格 → 选中行」和「测试」两节；做实机验证前读 `~/.claude/docs/DESKTOP_APP_TESTING.md`

## 背景

1. **选中行死循环**。`ASM_SPIKE_AUTOPLAY=1` 的第二步（`main.rs` 的 `autoplay`：清项目 → 30 天 → 选最忙项目 → 按 Cost 排序 → `select_row(0)`）之后，主线程 CPU 100%，`state.json` 被反复重写，画面不再更新，`make screenshot` 第 3 张停在第一步。`sample` 显示主线程一直在 `flush_effects` → 表格 `TableEvent` 订阅 → `AppRoot::on_table_event` → `select_row` → `persist` 里转（`tmp/2026-10-23-cost-context/diag/sample.txt`）。改动前的 commit `971e4b5` 在同一份数据上也复现，不是费用列引入的。
   - 上一个验收 agent 的推测（**未验证**）：`TableState::set_selected_row` 每调用一次就发一个 `SelectRow`；一次 update 里选了两次（`set_project` / `set_range` 各自走 `after_rows_changed` → `select_row`，最后又 `select_row(0)`），就排了两个事件；处理到旧的那个时选中被拉回旧行，又发一个事件，两个来回追。它的修法草稿和复现测试在 `/private/tmp/claude-501/-Users-reorx-Code-agent-session-manager/719f4771-094a-4b2a-864f-c2b5616fc48b/scratchpad/fix-src/crates/app/src/{main.rs,keyboard_tests.rs}`：`on_table_event` 只处理 `selected_row() == Some(ix)` 的 `SelectRow`。草稿没跑过，可以参考，不要照抄。
   - 手动操作会不会触发同样的循环没有验证。如果会，app 会卡死，这是本任务最重要的部分。
2. **parity 一直红**。`make parity` 在本机稳定失败 2 组（`list-code-30d`、`list-tenderbuddy`），与解析无关，样本在 `tmp/2026-10-23-python-parity/*.diff`：
   - 没有 started_at 的空 session 并列时，Rust 和 Python 排序不同
   - 只有 `memory/` 或 `sessions-index.json`、没有任何 transcript 的项目目录，Python 列成 `(0 sessions, 0 with transcript)`，`asm` 不列
   - 它一直红，真正的回归就会被淹没。另外 search 组偶尔因为比对期间有 session 正在写入而失败，重跑即过，这个不用处理

## 目标

app 里任何一次 update 里多次选中都不会让选中来回跳或卡死，`make screenshot` 三张图都正确；`make parity` 在本机全绿（除了正在写入的 session 导致的偶发失败）。

## 完成标准

1. `keyboard_tests.rs` 里有一个复现测试：修之前失败（不能把测试进程挂死，见「怎么做」），修之后通过
2. 列出代码里所有可能在一次 update 里选中两次的路径（至少看 `after_rows_changed`、`select_row`、`apply_filter`、`on_store_events`、`on_search_events`、autoplay 两个脚本），说明每条在修复后为什么安全；手动操作能触发的，补测试
3. `make screenshot` 的第 3 张显示「最忙项目 · 30 days · Cost 降序 · Metadata tab」，状态栏是 `autoplay: … sorted by Cost desc`；三张都用工具打开看过
4. `make parity` 跑两遍，除偶发的 search 组外全部 PASS
5. `cargo test --workspace` 全过，`cargo clippy --workspace --all-targets` 不新增警告
6. 报告写在 `tmp/2026-10-23-followups/report.md`，截图归档在 `tmp/2026-10-23-followups/`

## 已经定下的事

| 问题 | 决定 | 谁定的 |
|---|---|---|
| parity 差异往哪边统一 | 让 `asm` 跟 Python 一致，不改 `~/Code/scripts/claude_session_search.py`。AGENTS.md 写明 Python 脚本是行为基准 | 派出你的 agent，依据 AGENTS.md |
| 0 session 的项目目录 | `asm list` 也列出来，格式和 Python 一样。实现方式你定（例如 `resolve_project_keys` 时把 `projects/` 下的目录名并进来），但不要为此改索引 schema | 同上 |
| 无 started_at 的并列顺序 | 读 Python 脚本的 `list_sessions`（约 827 行），让 Rust 的顺序与它一致；如果 Python 的顺序本身依赖不确定的东西（如目录遍历顺序），在报告里说清楚，改为两边都确定的规则需要改 Python，那就停下来只报告 | 同上 |
| 修法是否沿用草稿 | 不强制。先确认根因再决定；如果根因是别的，按真实根因修 | 同上 |

## 范围

1. 选中行死循环：先写复现测试，确认根因，修，再审查其他路径
2. parity 的两处差异，按上表统一
3. （可选，前两项完成后再做）在页缓存热的情况下，各启动一次新代码和 `971e4b5` 的 debug app，冷索引（删掉 `$ASM_DB`），记录状态栏的 `first rows in …ms`，回答「费用列让冷启动首批行上屏变慢了多少」。旧版本在单独的 worktree / `CARGO_TARGET_DIR` 里编译，不要覆盖共享的 debug 二进制，用完删掉 worktree

## 不做的事

- M4 详情面板（等用户答复待确认项）
- 自算费用偏低的调查、未计价模型的标记、价格表
- 改 `api_usage` / schema / 索引器
- 顺手重构与本任务无关的代码

## 怎么做

- **测试先行**。复现测试不能让测试进程挂死：effect 循环会让 `run_until_parked` 永远不返回。可行的办法例如在测试里数 `on_table_event` 被调用的次数（加一个 `#[cfg(test)]` 计数器），或者把两次选中放进一次 update 后只 flush 有限次，自己选，报告里说明
- **不要碰真实索引**：所有起 app 的命令都 `export ASM_DB=$PWD/tmp/2026-10-23-followups/index.sqlite`（可以从 `tmp/2026-10-23-cost-context/app-index.sqlite` 复制一份），`make screenshot*` 自己会用一次性的 `state.json`。原因：真实索引是用户日常在用的
- **不要用 System Events / osascript / cliclick 发按键，不要动剪贴板**：全局按键会打进用户正在用的窗口。交互只用 autoplay、预设 `state.json`、cliclick 鼠标点击
- 截图前 `wake-screen`（`make screenshot*` 已包含）；结论以打开图片看到的为准
- 手动起的 app 用完就 kill，最后 `pgrep -fl agent-session-manager` 应为空，再跑 `mac-dev-cleanup --min-age 2`（还有别的 session 在用浏览器）

## 约束

- 可以改：`crates/app/src/` 下的代码与测试、`crates/core/src/bin/asm.rs`、`crates/core/src/index/db.rs`（只为列项目，不改 schema）、`crates/core/tests/cli.rs` 及 fixture 生成脚本（改了要 `uv run` 重新生成）
- 不要改：`AGENTS.md`、`kb/` 下的任何文件（包括本简报和 `kb/known-issues.md`）、`scripts/compare-with-python.sh` 的比对逻辑。这些由派出你的 agent 在看完报告后统一更新，避免和它冲突
- **不要 commit，不要 push**。改动留在工作区，派出你的 agent 会和文档一起做一次提交（用户要求代码和文档在同一个提交里）

## 交付

`tmp/2026-10-23-followups/report.md`，包含：

- 根因（附证据：测试输出、你加的日志或 sample），修法，以及为什么它不会吞掉用户真实的点击
- 「完成标准」第 2 条要求的路径清单
- parity 两遍的结果和改了什么
- 可选第 3 项的数字（没做就写没做）
- 改了哪些文件；`cargo test` / `clippy` 结果
- 发现但没处理的问题、你不同意本简报的地方
