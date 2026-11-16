# Known Issues

> 当前 app 已知、待后续处理的问题：缺陷、简化实现、未验收项、待建流程。只反映最新状态——解决了就删掉条目，经过记到 `kb/sessions/`。

## 窗口：700 pt 左右时会话表的状态栏文字重叠，详情面板被挤成半宽

- 记录：2026-11-17（usage-p4-accept 复验与 usage-p4-drill 返工时发现，`tmp/2026-11-12-usage-stats/usage-p4-accept/report.md`）
- 两个视图的工具栏在窄窗口下已经会换行，但窗口再窄到 700 pt 左右时，会话表底部状态栏的左右两段文字叠在一起，详情面板只剩半宽，标题和按钮被截掉。
- 处理方向：给窗口设最小宽度（例如 900 pt），或分别收拾状态栏和详情面板的窄屏布局。设最小宽度要用户定。

## 下钻：Usage 开着时点 session，搜索多半会被清空

- 记录：2026-11-17（usage-p4-drill 第二次返工的取舍，`kb/sessions/2026-11-16-usage-p4-drill.md`「协调者摘要（usage-p4-drill-r2）」）
- 从 Usage 点 session 跳回会话表时，只要有待重跑的搜索就清空搜索，免得重搜只取前 200 条时把刚选中的 session 挤掉。Usage 开着时每分钟扫描一次，有 session 在写就会重读表格，所以在 Usage 里待一会儿再点，搜索多半被清空（通知里写 `Search`）。另外，Active 过滤下会话表的 Model 下拉只列当过主模型的模型。
- 处理方向：让重搜一定保留目标 session（搜索请求带一个必须保留的 id，改 `search.rs`），或比较前后两次搜索的范围；Model 下拉在 Active 下按这段时间的调用数。

## 常驻菜单栏：菜单栏项被系统关掉后，关窗就找不到应用了

- 记录：2026-11-16（用户手动检查第 2 期时发现，`tmp/2026-11-12-usage-stats/status.md` 日志）
- macOS 26 上用户把状态项拖出菜单栏（或在 Thaw 这类工具里拖动时误拖出去），系统会关掉「系统设置 › 菜单栏 › 允许在菜单栏中显示」里的本应用（Control Center 的 `trackedApplications` 里 `isAllowed: false`），之后每次启动都不显示，Thaw 里也看不到。这时按 `⌘W` 关窗，Dock 图标也跟着消失，应用没有任何可见入口，只能从 Finder 双击或 `open` 把窗口叫回来。
- 处理方向：关窗前判断状态项是否真的显示着（macOS 26 是否给得出这个信号要实测），不显示就保留 Dock 图标或直接退出；至少在说明框里写一句怎么在系统设置里恢复。

## 发版：`make notarize` 不能重跑

- 记录：2026-11-16（usage-p2-accept2 跑 `make release-local` 时发现）
- dmg 公证遇到网络超时后，重跑 `make notarize` 会因为 dmg 已经签过名直接失败，只能从头出包。
- 处理方向：签名前检查是否已签名（或先删掉旧签名），让 notarize 这一步可以单独重跑。

## 费用：同一模型带日期和不带日期的名字混用时，主模型可能判错

- 记录：2026-11-16（usage-p3-chart 返工时发现，`kb/sessions/2026-11-16-usage-p3-chart.md`「协调者摘要（usage-p3-app-r1）」）
- core `cost::primary_model` 按原始模型名分组算花费，同一个 session 里 `claude-haiku-4-5-20251001` 和 `claude-haiku-4-5` 都出现时，花费被拆成两组，主模型（Cost tab、按模型换算、会话表的 Model 过滤）可能落到另一个模型上。很少见。
- 处理方向：分组前先 `pricing::normalize`，会改主模型的结果，要提升 `SCHEMA_VERSION` 或确认它不进索引。

## 工具栏：Custom 日期选择器的默认区间跨月后不更新

- 记录：2026-11-16（usage-p1-app 发现，`kb/sessions/2026-11-16-usage-p1-app.md`「交给协调者」）
- 没选过 Custom 时，日期选择器默认显示「本月 1 日到今天」，这个默认值只在启动时算一次。应用连续开过月初（用量统计第 2 期起关窗不退出，会更常见）后，打开 Custom 看到的还是上个月。时间预设本身会在跨过 4:00 时重算（`roll_day`），不受影响。
- 处理方向：`roll_day` 换日时一并重算 `RangePreset::default_custom` 的默认区间（用户没手动选过时）。

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
- 少数响应的 `usage` 顶层 `input` / `cache_read` / `output` / `cache_creation_input_tokens` 全为 0，真实数字只在 `usage.iterations[0]` 里（到 2026-11-16 本机共 4 次：d074913f、01dacff6、603da7ec、196366dd，都是 Opus 5.5，读缓存约 36 万 token）。`parse.rs::tokens_of` 只读顶层：这次响应只按 `cache_creation` 细分计了缓存写入，读缓存和输出没计价；`conversation.rs::cache_check` 把它当成「读回 0」报一次失效（不明），又因为它的 prompt 只剩几千 token，紧接着那次真正只读回约 29k 的失效被漏掉。
- 处理方向：`iterations` 非空时按各项求和（官方语义下服务端 compaction 的那一项也要计费，见 `kb/refs/claude-api-compaction-threshold.md`），失效判定取最后一个 `message` 项；改解析后提升 `SCHEMA_VERSION`。方案见 `kb/plans/2026-11-16-cache-break-accuracy.md`。

## 回合卡：`/rewind` 之后的第一次调用被标成缓存失效

- 记录：2026-11-12（`tmp/cache-breaks-causes.py`，本机 60 天 38 次「不明」里 5 次）
- `cache_check` 按文件顺序和上一次调用比。rewind 后新消息的 `parentUuid` 指回更早的节点，这次调用只读回 rewind 点之前的前缀，被丢掉的那段算进了「少读」，回合卡显示一次原因不明的失效。Claude Code 文档说明 rewind 会命中更早的缓存条目，不是失效（`kb/refs/claude-code-prompt-caching.md`「Rewinding the conversation」）。
- 处理方向：沿 `parentUuid` 找到这次调用的祖先调用，和它的 prompt 比；compact 之后链会断开，仍按 compact 处理。方案见 `kb/plans/2026-11-16-cache-break-accuracy.md`。

## 费用：价格表写在代码里，新模型上线后它的调用不计价，表格不提示

- 记录：2026-10-23（有意的简化；`crates/core/src/pricing.rs`）
- 价格表按 2026-10-23 的官方价格页写死。Claude Code 用上表里没有的模型后，这些调用的费用不计入，表格里的 Cost 只显示其余部分，看不出少了；只有详情 Metadata 的 Unpriced 一行列出模型名。`asm cost` 会在末尾列出未计价的模型。
- Context 窗口自 2026-11-03 起优先读取本机模型目录，不再只依赖写死的表；价格仍然写死，见 `kb/plans/2026-11-03-context-window-from-catalog.md`。
- 处理方向：表格的 Cost 单元格在有未计价调用时加标记；新模型发布后更新 `pricing.rs` 并提升 `SCHEMA_VERSION`。

## Context：目录声明的窗口可能与会话实际窗口不同

- 记录：2026-11-03（有意的简化；`kb/plans/2026-11-03-context-window-from-catalog.md`）
- Ctx 使用当前缓存声明的窗口，不还原会话当时的账号、`CLAUDE_CODE_DISABLE_1M_CONTEXT`、Claude Code 内置上限或 GrowthBook 调整。受这些条件影响时，表格、Metadata 和 CLI 虽然一致，百分比仍可能低于或高于实际占用；本机当前 8 种模型的目录与内置窗口一致。
- 后续判断依据：出现真实的窗口不一致样本时核对运行时口径；当前保持只读本机目录和静默回退。
