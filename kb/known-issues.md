# Known Issues

> 当前 app 已知、待后续处理的问题：缺陷、简化实现、未验收项、待建流程。只反映最新状态——解决了就删掉条目，经过记到 `kb/sessions/`。

## 用量：重建索引后，记录文件已被删掉的 session 的用量永久丢失

- 记录：2026-11-16（设计用量统计页时在索引副本上核对；`kb/plans/2026-11-16-usage-stats/index.md`）
- Claude Code 会删掉旧记录（本机 `cleanupPeriodDays` 是 60），用户也会手动删。记录没了以后，增量扫描仍保留这个 session 的用量（`indexer.rs::mark_missing`），但任何一次重建都只重读磁盘上还在的文件，这些用量就没了。会触发重建的有：提升 `SCHEMA_VERSION`（改价格、改解析都要提升）、schema 变化、Index → Rebuild、索引损坏后自动删除重建、根目录变化。2026-11-16 的索引里有 65 个这样的 session。会话表的费用和以后的用量统计都会因此少算。
- 账本上线前，这次重建大概率会先发生：2026-11-16 工作区里有一项未提交的 `asm` 改动把 `SCHEMA_VERSION` 从 5 提升到 6，合并后用新构建打开真实 `index.sqlite`（`make dev`、截图脚本、发版）就会重建。`asm` 用的是自己的 `index-cli.sqlite`，不受影响。
- `tmp/2026-11-12-usage-stats/index-copy.sqlite` 是 2026-11-16 10:14 的真实索引备份，里面有这批用量（2,225 个有用量的 session，56 个记录已删）。账本上线并导入它之前不要删。
- 处理方向：单独的用量账本，重建和删除记录文件都不动它，重建前先把旧索引的用量抄进账本；备份的导入是它的实施步骤 7。见 `kb/plans/2026-11-16-usage-stats/usage-ledger.md`。

## 打包：从 Finder / Dock 启动的 app 读不到 shell 里的环境变量

- 记录：2026-10-28（1.0 打包时核对代码发现，未实测）
- `.app` 由 launchd 启动，只有系统默认的环境，不带 `~/.zshrc` 里设的变量。两处受影响：`CLAUDE_CONFIG_DIR`（`crates/core/src/claude/paths.rs`）设了的用户，app 仍然读 `~/.claude`；`PATH` 里没有 `/opt/homebrew/bin`，`regex.rs` 找不到 `shfmt`，长 shell 命令不再折行（会安静地退回原样显示）。从终端运行二进制、`asm` CLI 不受影响。
- 处理方向：Claude 目录做成 app 里的设置（和收藏目录一样存进 `config.json`）；`shfmt` 额外查 `/opt/homebrew/bin`、`/usr/local/bin`。

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

## 搜索：正则扫描远超 500ms 目标，30 天的数据要 6–8 秒

- 记录：2026-09-28（复测 M3 的数字时发现；`kb/sessions/2026-09-28-m3-known-issues.md`）；2026-11-14 复测
- 2026-09-28 本机 2.0GB 时，app 探针量到只搜 User + Assistant 250–310ms、加上 Tools 460–530ms，PRD 目标 500ms 以内。2026-11-14 数据已到 7.4GB，用 CLI 的正则搜索（`asm search` 无 `--index`）量到约 1 秒/GB：7 天 1.6GB 2.4s，30 天 5.1GB 6.0–7.7s，只搜 User + Assistant 与全搜一样慢，时间花在逐行解析 JSON 上（当时机器 load 5–6）。app 的正则模式用同一套逐行解析，推测同样慢，未在 app 里量。结果是流式出现的，首批行远早于结束。
- 处理方向：每行先在原始字节上做字面量预过滤，通过了再解析 JSON；整份文件不含查询词时直接跳过。`kb/plans/2026-11-15-asm-skill-and-cli/session-search.md` 先在新的 `asm search` 上做，旧路径（app、`search-old`）之后再接，改完必须跑 `make parity`。

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
- Cost 列按 transcript 里每次 API 响应的 usage 计价。2026-11-12 在 1,954 个 Claude Code 自己记了费用（`cost-state`）的 session 上，97.6% 与它相差不到 10%，34 个自算偏低超过 10%，合计偏低 1.95%。Claude Code 文档写明 compact 的摘要请求（会把整段历史读一遍）、session 标题和部分辅助请求是主对话之外的独立请求（`kb/refs/claude-code-prompt-caching.md`），它们不写进 transcript；各类各占多少未量过。按项目或月份看合计时，结果会比实际账单略低。
- 处理方向：找一个偏低的 session，对照 `cost-state.modelUsage` 确认缺的是哪类调用；compact 的摘要请求可按 compact 次数和当时的 context 大小补估。

## 费用与缓存失效：顶层 usage 全为 0 的响应漏算费用，回合卡误报一次失效、漏报下一次

- 记录：2026-11-12（对照官方 prompt caching 文档核对用量解析时发现；`tmp/cache_audit.py`、`tmp/cache-breaks-causes.py`）
- 少数响应的 `usage` 顶层 `input` / `cache_read` / `output` / `cache_creation_input_tokens` 全为 0，真实数字只在 `usage.iterations[0]` 里（本机 60 天 3 次，都是 Opus 5.5，读缓存约 36 万 token）。`parse.rs::tokens_of` 只读顶层：这次响应只按 `cache_creation` 细分计了缓存写入，读缓存和输出没计价；`conversation.rs::cache_check` 把它当成「读回 0」报一次失效（不明），又因为它的 prompt 只剩几千 token，紧接着那次真正只读回约 29k 的失效被漏掉。
- 处理方向：`iterations` 非空时按各项求和（官方语义下服务端 compaction 的那一项也要计费，见 `kb/refs/claude-api-compaction-threshold.md`），context 和失效判定取最后一个 `message` 项；改解析后提升 `SCHEMA_VERSION`。

## 回合卡：`/rewind` 之后的第一次调用被标成缓存失效

- 记录：2026-11-12（`tmp/cache-breaks-causes.py`，本机 60 天 38 次「不明」里 5 次）
- `cache_check` 按文件顺序和上一次调用比。rewind 后新消息的 `parentUuid` 指回更早的节点，这次调用只读回 rewind 点之前的前缀，被丢掉的那段算进了「少读」，回合卡显示一次原因不明的失效。Claude Code 文档说明 rewind 会命中更早的缓存条目，不是失效（`kb/refs/claude-code-prompt-caching.md`「Rewinding the conversation」）。
- 处理方向：沿 `parentUuid` 找到这次调用的祖先调用，和它的 prompt 比；compact 之后链会断开，仍按 compact 处理。

## 费用：价格表写在代码里，新模型上线后它的调用不计价，表格不提示

- 记录：2026-10-23（有意的简化；`crates/core/src/pricing.rs`）
- 价格表按 2026-10-23 的官方价格页写死。Claude Code 用上表里没有的模型后，这些调用的费用不计入，表格里的 Cost 只显示其余部分，看不出少了；只有详情 Metadata 的 Unpriced 一行列出模型名。`asm cost` 会在末尾列出未计价的模型。
- Context 窗口自 2026-11-03 起优先读取本机模型目录，不再只依赖写死的表；价格仍然写死，见 `kb/plans/2026-11-03-context-window-from-catalog.md`。
- 处理方向：表格的 Cost 单元格在有未计价调用时加标记；新模型发布后更新 `pricing.rs` 并提升 `SCHEMA_VERSION`。

## Context：目录声明的窗口可能与会话实际窗口不同

- 记录：2026-11-03（有意的简化；`kb/plans/2026-11-03-context-window-from-catalog.md`）
- Ctx 使用当前缓存声明的窗口，不还原会话当时的账号、`CLAUDE_CODE_DISABLE_1M_CONTEXT`、Claude Code 内置上限或 GrowthBook 调整。受这些条件影响时，表格、Metadata 和 CLI 虽然一致，百分比仍可能低于或高于实际占用；本机当前 8 种模型的目录与内置窗口一致。
- 后续判断依据：出现真实的窗口不一致样本时核对运行时口径；当前保持只读本机目录和静默回退。
