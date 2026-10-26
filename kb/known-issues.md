# Known Issues

> 当前 app 已知、待后续处理的问题：缺陷、简化实现、未验收项、待建流程。只反映最新状态——解决了就删掉条目，经过记到 `kb/sessions/`。

## CLI：`asm --db` 帮助文本里的默认索引路径与实际不符

- 记录：2026-09-28（精简 AGENTS.md 时核对代码发现）
- `crates/core/src/bin/asm.rs` 的 `--db` 说明写的是 `~/Library/Application Support/AgentSessionManager/index.sqlite`，实际默认值来自 `claude/paths.rs` 的 `ProjectDirs::from("dev", "reorx", "AgentSessionManager")`，即 `~/Library/Application Support/dev.reorx.AgentSessionManager/index.sqlite`。只影响 `--help` 的显示，行为正确。
- 处理方向：改帮助文本；CLI 测试若断言了 help 输出要一并更新。

## 打包：已定的 bundle id 与数据目录的限定符不一致

- 记录：2026-09-28（精简 AGENTS.md 时核对代码发现）
- PRD 已定 bundle id 为 `com.reorx.agent-session-manager`，而索引与 `state.json` 所在目录是 `dev.reorx.AgentSessionManager`。目前还没有 `.app` bundle，所以没有实际影响；M5 用 cargo-bundle 打包后，app 的标识与它的数据目录会对不上。
- 处理方向：M5 打包前决定是否统一。改数据目录名会让现有索引和视图状态落到新路径，索引可重建，`state.json` 需要迁移或接受重置。

## 表格与详情：Prompts 列的数字比详情里的回合数多

- 记录：2026-10-27（M4a 验收；总 plan D7，`kb/sessions/2026-10-27-asm-m4a-accept.md`）
- Prompts 列沿用 Python 脚本的口径（`make parity` 逐字比对）：非 sidechain、非 isMeta、有文字的 user 记录都算一条（`parse.rs::is_human_prompt`）。详情只从真实输入开始编号回合，下面这些在 Prompts 里算、在详情里不是回合：打断标记 `[Request interrupted by user]`（详情记为上一回合的 interrupted）、compact 摘要（详情是分隔线上的 summary）、task-notification 等非人工消息（详情是不编号的 AUTO 卡）。本机例子：`481e870a` Prompts 2、详情 1 turn，多出的是打断标记；`35fec037` Prompts 21、详情 13 turns · 4 auto，另 4 条是 compact 摘要。
- 影响：按 Prompts 排序或估计对话长短时数字偏大，表格和详情并排时看上去对不上。
- 处理方向：Prompts 改口径要和 Python 脚本一起改（脚本是行为基准）；不改的话至少在表头提示或术语里说明它数的是 user 记录。M4 不动。

## 详情：Read 图片的步骤看不出读到了什么

- 记录：2026-10-27（`kb/sessions/2026-10-26-asm-m4a-store.md`"遗留"；M4a 验收在 `5f6eea99` 上确认）
- core 的 `ToolResult` 只记文字输出，工具结果里的图片 block 被丢掉。Read 一张图片的步骤结果摘要是空的，展开后只有 input、没有 output，看不出读没读成功、图多大。`5f6eea99` 的 112 次工具调用里有 79 次是这样。
- 处理方向：core 解析工具结果时像 prompt 附件一样记下图片（`ToolResult` 加 attachments），app 在结果摘要和 output 里显示 `image · 230 KB`。

## 详情：记录文件不在页缓存里时，打开 70MB 的 session 首帧约 150–160ms，偶发超过 200ms

- 记录：2026-10-27（M4a 验收；`kb/sessions/2026-10-27-asm-m4a-accept.md`"实测数字"，数据在 `tmp/2026-10-27-m4a-turn-cards/accept/timing/cold-*.log`）
- 200ms 的目标是在页缓存热时量的（release 45–79ms）。先用 `tmp/2026-10-27-m4a-turn-cards/accept/evict.py` 把 `5f6eea99` 逐出页缓存再打开：release 首帧 152 / 161 / 356ms，时间几乎全在读取（119–335ms）。本机这个文件读过一两分钟后再查就已经不在页缓存里，所以打开一个好一阵没看的大 session 多半是冷的，用户会看到骨架停留一下。只量了这一个文件。
- 处理方向（推测，未验证）：映射后 `madvise(MADV_SEQUENTIAL | MADV_WILLNEED)` 让内核预读，或者改成大块顺序 `read`；先在 `asm session --conversation` 上对比冷读取耗时。

## 表格：Size 列在 1000 MB 以上时首位被截掉

- 记录：2026-10-27（M4a 验收时在合成的 4GB 记录文件上看到；`tmp/2026-10-27-m4a-turn-cards/accept/states-shots/crop-size.png`）
- `model.rs::format_size` 只有 KB 和 MB 两档，4 GiB 的文件写成 `4096.0 MB`，60px 宽的 Size 列放不下，左边被截掉，显示成 `096.0 MB`，看上去比实际小 40 多倍，按 Size 排序的结果却是对的。本机现在最大的记录文件 70MB，还没有真实 session 碰到。
- 处理方向：1000 MB 起改写成 `1.0 GB`。

## 表格与详情：粘贴内容的 `<pasted_content id="…">` 标签原样显示

- 记录：2026-10-27（M4a 验收；`tmp/2026-10-27-m4a-turn-cards/accept/r1/c/1-conversation.png`）
- 用户粘贴的长文本在记录里包在 `<pasted_content id="9c86">…</pasted_content>` 里。表格 Preview 列、详情标题和回合卡 prompt 的第一行都是这个标签，真正的内容要往下看；本机 2061 个 session 里有 72 个的第一条 prompt 以它开头（`tmp/2026-10-27-m4a-turn-cards/accept/pasted-count.py`）。
- 处理方向（推测）：显示时剥掉标签外壳、保留内容，必要时加一个 `pasted` 小标签。First prompt / Preview 的文字也进 `asm list` 的输出，和 Python 脚本逐字比对（`make parity`），所以只在 app 显示层改，或者两边一起改。

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
