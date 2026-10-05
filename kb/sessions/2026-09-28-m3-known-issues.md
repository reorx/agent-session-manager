---
created: 2026-09-28
tags:
  - m3
  - search
  - performance
  - keyboard
  - focus
  - testing
  - agent-session-manager
---

# 处理 M3 笔记里的已知问题：复测搜索耗时，补键盘测试，修复启动后快捷键无效

## 概要

M3 笔记的"已知问题与未做"有四条：两字查询走 `LIKE` 慢、正则冷扫描 1.6s、详情面板看不到工具调用里的命中、Escape 没有用真实按键验证。第三条属于 M4，没有动。其余三条逐条复测和处理，结果是：

- **两条性能问题的数字都量错了。** 两字查询在 debug app 里稳定 16–20ms，正则首次扫描在页缓存已热时 260–310ms。笔记里的 83–137ms 和 1.6s 出自 `make screenshot-search`，而这个脚本会先唤醒屏幕，唤醒后的十几秒里 app 内的耗时可以到平时的 4–8 倍。代码没有为这两条做改动。
- **Escape 的验证改成了自动化测试。** 新增 `keyboard_tests.rs`：整个应用跑在无界面的测试窗口里，按键走 GPUI 的分发。Escape、Enter、`n` / `shift-n`、`cmd-g`、`cmd-f` 都有断言。
- **写测试时发现一个真实缺陷并修复。** 窗口刚打开时没有任何元素有焦点，快捷键和菜单动作到不了 `AppRoot`，用户必须先点一下表格或搜索框。真实窗口里用探针确认过（`FocusSearch available = false`）。修复是启动时把焦点给表格。

### 耗时对照

索引副本：17.6k 条消息、21MB 文本；transcript 1278 份、2.0GB。debug 构建，单位 ms。

| 运行 | 先唤醒了屏幕 | 两字查询"搜索" | 正则首次扫描 | 正则 + Tools |
|---|---|---|---|---|
| before | 是 | 84 | 1510 | 573 |
| before2 | 调用了 `wake-screen`，是否真的唤醒没有留下记录 | 87 | 1874 | 502 |
| wake2 | 是 | 19 | 1115 | 552 |
| after | 是 | 74 | 2269 | 528 |
| wake1 | 屏幕本来就亮着 | 18 | 290 | 929 |
| trace | 否 | 18 | 969 | 495 |
| nonap | 否 | 20 | 309 | 529 |
| plain | 否 | 18 | 264 | 502 |
| nowake1 | 否 | 30 | 272 | 512 |
| nowake2 | 否 | 19 | 258 | 504 |
| 探针（不起窗口，每项 5 次） | 否 | 16–17 | 252–285 | 457–519 |
| `asm` release，新进程 | 否 | 14.7 | 约 260（整个进程） | — |

确认唤醒过屏幕的 3 次运行（before、wake2、after），正则首次扫描全部在 1.1s 以上，两字查询有 2 次偏慢。没有唤醒的 6 次里 5 次正常，`trace` 那次的 969ms 和 `wake1` 的 929ms 没有解释。所以"唤醒屏幕"和"变慢"之间是相关，机制没有查明。

逐批计时显示慢的那次扫描是前 14 批（最新的文件）每批 27–125ms，之后每批 3–17ms；`sample` 显示扫描线程的时间在 `find_in_bytes` 里，不是在等锁。页缓存当时是热的：同一时刻 `asm` 新进程扫完全部文件只要 0.26s。

### 为什么不做 bigram 索引和预读

- 两字查询 16–20ms，目标是 100ms。耗时随消息文本量线性增长，按现在的速度要到 100MB 以上才会碰线。
- 预读 transcript 针对的是磁盘冷读，而复测没有量到磁盘冷读：慢的那几次页缓存都是热的。真正的冷缓存耗时需要 `sudo purge` 才能量，没有做。

## 修改的文件

- `crates/app/src/main.rs` — 启动时把焦点给表格；`AppRoot::new` 改为接收 `AppPaths`（数据目录、索引、视图状态三个路径），环境变量只在 `AppPaths::from_env` 里读；快捷键绑定移到 `bind_keys`，测试和 `main` 共用
- `crates/app/src/keyboard_tests.rs` — 新增，3 个行为测试
- `crates/app/src/search.rs` — 新增 `real_index_timings`（`#[ignore]`），在真实索引上打印文本与正则搜索的耗时
- `crates/app/src/toolbar.rs` — `SEARCH_DEBOUNCE` 改为 crate 内可见，测试用它推进时钟
- `crates/app/Cargo.toml`、`Cargo.lock` — `[dev-dependencies]` 加 `gpui-kit = "=0.6.1"` 的 `test-support`；lock 只新增了测试用的包，已有包的版本没有变
- `AGENTS.md` — 测试命令、量耗时的命令、焦点规则、键盘测试规则
- `kb/known-issues.md` — 新增 4 条
- `kb/docs/app-architecture.md` — 新增"键盘焦点"一节
- `kb/docs/gpui-kit-guide.md` — 测试一节重写，加上无焦点的说明和"截图脚本的耗时不能当性能数字"
- `kb/docs/product-prd.md`、`kb/plans/2026-09-08-agent-session-manager-prd.md`、`kb/notes/2026-09-21-m3-search.md` — 更正耗时数字

## 注意事项

- **量性能不要借用截图脚本。** 它为了截图要唤醒屏幕，唤醒本身就改变了被测的东西。用探针：`ASM_BENCH_DB=<索引副本> cargo test -p agent-session-manager real_index_timings -- --ignored --nocapture`。
- **一次性的 autoplay 只能量到"第一次"。** 脚本里每种查询只跑一遍，量到的是首次查询。要知道稳定值得在同一个进程里重复跑。
- **在无界面窗口里跑整个应用是可行的。** 要点有三个：`cx.executor().allow_parking()` 让测试调度器接受真线程的唤醒；等后台结果用真实时间轮询，`wait_for` 走测试时钟等不到；防抖定时器用 `advance_clock` 推进。
- **测试要先证明自己能失败。** 把 Escape 的处理者换成空函数后，Escape 测试在"搜索框被清空"这一条上失败，说明它测到了东西。
- **`asm` 会在正常退出时合并 WAL。** 本次第一条 `make asm ARGS="stats"` 打开了真实索引，退出时 SQLite 把 2.6MB 的 WAL 合并进主文件并删掉了 `-wal` / `-shm`。这是 SQLite 的正常行为，数据没有变化，但索引文件的修改时间变成了今天。之后的所有操作都在 `tmp/` 下的副本上进行。
- 验收用的副本会被 app 的启动扫描写入。要保持副本不变，每次从真实索引重新拷贝。

## 遗留问题

已同步到 `kb/known-issues.md`：

- 正则带 Tools 范围的扫描 460–530ms，已到 500ms 目标线，随数据量增长
- 刚唤醒屏幕后查询耗时偏高 4–8 倍，原因未查明（推测是线程调度，未验证）
- 正则扫描的冷缓存耗时没有量过
- 一到两个字的查询是全表扫描，文本超过 100MB 需要重新考虑

没有处理、仍在 `kb/known-issues.md` 里的：详情面板看不到工具调用与 thinking 里的命中（M4 步骤 4）。

键盘测试覆盖不到系统把物理按键交给 GPUI 的那一段。这一段是 GPUI 的职责，M3 时用真实按键验证过 Enter 和 `n`，没有单独登记。

## 已解决的已知问题

这两条登记在 M3 笔记里，没有进过 `kb/known-issues.md`：

- **Escape 清空只经 autoplay 验证，没有用按键确认** — `keyboard_tests.rs` 的 `escape_in_the_search_box_drops_the_search_and_hands_the_focus_to_the_table` 从 `cmd-f`、打字、等搜索结果到按 Escape 走完整条路径，断言搜索框清空、焦点回到表格、Matches 列消失、行数恢复
- **启动后快捷键和菜单无效，直到用户点击**（本次发现）— `AppRoot::new` 末尾把焦点给表格。测试 `the_table_has_the_focus_after_launch_so_shortcuts_work_before_any_click`；真实窗口里探针从 `focused = false / FocusSearch available = false` 变为 `true / true`

## 验收

- `make test`：core 71 个、app 58 个通过，1 个 ignored（探针）
- 键盘测试连跑 5 次全部通过，单次 0.11–0.14s
- `make screenshot-search` 7 张截图已逐张看过，界面与修复前一致，表格没有多出焦点框
- 截图与日志：`tmp/2026-09-28-m3-known-issues/`（`before/`、`after/` 是截图，`runs/` 是各次运行的日志，`sample/` 是采样结果）

## 相关文档

- [M3：界面搜索](../notes/2026-09-21-m3-search.md) — 本次处理的问题清单来自它的"已知问题与未做"，已更新
- [运行时架构](../docs/app-architecture.md) — 更新，新增"键盘焦点"
- [gpui-kit 开发指南](../docs/gpui-kit-guide.md) — 更新，测试一节与截图脚本一节
- [产品 PRD](../docs/product-prd.md) — 更新状态表里的搜索耗时与快捷键说明
- [原始 PRD](../plans/2026-09-08-agent-session-manager-prd.md) — 更正 M3 验收记录里的两字查询耗时
- [M4 详情面板方案](../plans/2026-09-28-m4-detail-panel.md) — 参考，确认"工具调用内的命中"归它的步骤 4
