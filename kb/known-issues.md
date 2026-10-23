# Known Issues

> 当前 app 已知、待后续处理的问题：缺陷、简化实现、未验收项、待建流程。只反映最新状态——解决了就删掉条目，经过记到 `kb/sessions/`。

## 回归验证：parity 因空 session 的并列顺序一直有 1 组失败

- 记录：2026-10-10（`kb/notes/2026-10-10-transcript-reader.md`），2026-10-23 查明原因（`tmp/2026-10-23-followups/report.md`）
- `make parity` 的 `list-code-30d` 里，两个都没有 started_at 的空 session 顺序相反，其余 20 组通过。原因在 Python 参考脚本：它按 `glob('*.jsonl')` 的目录遍历顺序收集 transcript，之后只按 `started` 做稳定排序，没有次关键字，所以并列项的顺序取决于文件系统；Rust 按 updated 降序。只要它一直红，真正的回归就看不出来。另外 search 组偶尔因为比对期间有 session 正在写入而失败，重跑即过。
- 处理方向：定一个两边共用的并列排序键（例如 started 降序、updated 降序、id），同时改 Python 脚本（`~/Code/scripts`，不在本仓库）和 Rust，并补测试。要改参考脚本，需要用户同意。

## CLI：`asm --db` 帮助文本里的默认索引路径与实际不符

- 记录：2026-09-28（精简 AGENTS.md 时核对代码发现）
- `crates/core/src/bin/asm.rs` 的 `--db` 说明写的是 `~/Library/Application Support/AgentSessionManager/index.sqlite`，实际默认值来自 `claude/paths.rs` 的 `ProjectDirs::from("dev", "reorx", "AgentSessionManager")`，即 `~/Library/Application Support/dev.reorx.AgentSessionManager/index.sqlite`。只影响 `--help` 的显示，行为正确。
- 处理方向：改帮助文本；CLI 测试若断言了 help 输出要一并更新。

## 打包：已定的 bundle id 与数据目录的限定符不一致

- 记录：2026-09-28（精简 AGENTS.md 时核对代码发现）
- PRD 已定 bundle id 为 `com.reorx.agent-session-manager`，而索引与 `state.json` 所在目录是 `dev.reorx.AgentSessionManager`。目前还没有 `.app` bundle，所以没有实际影响；M5 用 cargo-bundle 打包后，app 的标识与它的数据目录会对不上。
- 处理方向：M5 打包前决定是否统一。改数据目录名会让现有索引和视图状态落到新路径，索引可重建，`state.json` 需要迁移或接受重置。

## 详情：索引扫描进行中选中一行，对话要等扫描结束才出现

- 记录：2026-09-28（M4 方案调研时读代码发现；`kb/plans/2026-09-28-m4-detail-panel.md` §2）
- store 线程是单循环，`StoreRequest::Messages` 排在扫描后面。冷启动或 Rebuild 的 3.6–4s 内选中行，Conversation 一直是空的。
- 处理方向：M4 步骤 2 给 transcript 读取单开一条线程。

## 详情：Conversation 只显示索引里的前 300 条 user / assistant 文本，每条截到 4000 字

- 记录：2026-09-28（M2 的过渡实现，有意为之；`kb/notes/2026-09-21-m2-real-data.md`）
- 该视图没有虚拟化，全量渲染上千条会卡，所以设了上限。工具调用、thinking 不显示，没有 Markdown 渲染。搜索命中落在上限之外时靠两个补丁移动窗口和截断位置。
- 处理方向：M4 步骤 1–3 直接读 transcript 并虚拟化，届时删掉上限和两个补丁。

## 搜索：工具调用与 thinking 里的命中在详情面板里看不到

- 记录：2026-09-28（M3 遗留；`kb/notes/2026-09-21-m3-search.md`"已知问题与未做"）
- 正则 + Tools 范围或选了 Labels 时，命中只在表格的片段里可见，详情面板底部有一句说明。
- 处理方向：M4 步骤 4。

## 搜索：正则模式带 Tools 范围的扫描已到 500ms 目标线

- 记录：2026-09-28（复测 M3 的数字时发现；`kb/sessions/2026-09-28-m3-known-issues.md`）
- 本机 1278 份 transcript、2.0GB。正则只搜 User + Assistant 是 250–310ms，加上 Tools 是 460–530ms，PRD 的目标是 500ms 以内。M3 验收时（950 份、1.27GB）是 380–425ms，耗时随数据量线性增长。结果是流式出现的，首批行远早于结束。
- 处理方向：每行先在原始字节上做字面量预过滤，通过了再解析 JSON。正则匹配的是 JSON 解码后的文本，预过滤要处理转义，改完必须跑 `make parity`。

## 搜索：刚唤醒屏幕后的十几秒里查询耗时可以到平时的 4–8 倍，原因没有查明

- 记录：2026-09-28（`kb/sessions/2026-09-28-m3-known-issues.md` 的对照表）
- `wake-screen` 把睡着的屏幕唤醒后立刻跑 app：两字搜索 74–87ms（平时 17–20ms），正则首次扫描 1.1–2.3s（平时 0.26–0.31s），第 13 秒左右恢复正常。确认唤醒过的 3 次运行，正则扫描全部变慢，两字搜索 2 次变慢；另有 2 次没有唤醒的运行也各出现了一次约 1s 的扫描，没有解释。采样显示时间花在扫描线程上而不是等锁，推测是系统在这段时间里压低了 app 线程的调度，未验证。
- 影响：`make screenshot-search` 日志里的耗时不能当性能数字；用户唤醒电脑后马上搜索也可能遇到。
- 处理方向：量耗时用 `real_index_timings` 探针。要查原因的话，在唤醒后用 `sample` 和 `powermetrics` 对比线程落在哪类核心上。

## 搜索：正则扫描在页缓存是冷的时候要多久，没有量过

- 记录：2026-09-28（M3 笔记把 1.6s 记成了冷缓存耗时，复测发现那是上一条的现象）
- 清空页缓存需要 `sudo purge`，验收环境里没有做。重启后第一次正则搜索要从磁盘读 2GB，耗时未知。
- 处理方向：手动 `sudo purge` 后跑一次探针。如果超过 1s，切到正则模式时在后台预读 transcript。

## 搜索：一到两个字的查询是全表扫描，耗时随消息文本量线性增长

- 记录：2026-09-28（有意为之的简化实现；`kb/notes/2026-09-21-m3-search.md`）
- 全文索引按三个字符切词，更短的词查不了，改走 `LIKE`。现在 21MB 消息文本要 16–20ms，离 100ms 的目标还有 5 倍余量。
- 处理方向：消息文本超过 100MB 再考虑。届时可选另建两字索引，或把文本读进内存扫描。

## 费用：自算费用不含记录文件之外的 API 调用，个别 session 偏低 10% 以上

- 记录：2026-10-23（`kb/plans/2026-10-23-cost-context-columns.md`）
- Cost 列按 transcript 里每次 API 响应的 usage 计价。在 973 个 Claude Code 自己记了费用（`cost-state`）的 session 上，96% 与它相差不到 10%，但有 22 个自算偏低超过 10%，合计偏低约 1%。推测是标题生成、auto mode 分类器、compact 等不写进 transcript 的调用，未验证。按项目或月份看合计时，结果会比实际账单略低。
- 处理方向：找一个偏低的 session，对照 Claude Code 的调用记录确认缺的是哪类调用；如果是固定开销，可以按调用次数补估。

## 费用：价格表写在代码里，新模型上线后它的调用不计价，表格不提示

- 记录：2026-10-23（有意的简化；`crates/core/src/pricing.rs`）
- 价格表按 2026-10-23 的官方价格页写死。Claude Code 用上表里没有的模型后，这些调用的费用不计入，表格里的 Cost 只显示其余部分，看不出少了；只有详情 Metadata 的 Unpriced 一行列出模型名。`asm cost` 会在末尾列出未计价的模型。
- 处理方向：表格的 Cost 单元格在有未计价调用时加标记；新模型发布后更新 `pricing.rs` 并提升 `SCHEMA_VERSION`。

## 表格：点表头排序后，选中高亮停在原来的行号上，和详情面板对不上

- 记录：2026-10-23（选中事件修复时读源码发现；`tmp/2026-10-23-followups/report.md`），**未实机验证**
- gpui-component 表格的表头排序（`TableState::perform_sort`）只重排 delegate 的行，不改 `selected_row`；`AppRoot` 对表格的 observe 只保存视图状态。所以点表头后，高亮停在原来的行号上，那一行已经换成了另一个 session，详情面板仍显示原来的 session。和「选中跟着 session 走」的规则不一致。
- 处理方向：先用 cliclick 点表头实机确认；确认后在表格排序变化时按 session id 重新定位选中（走 `after_rows_changed`），并在 `keyboard_tests.rs` 补测试。

