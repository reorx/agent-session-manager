---
created: 2026-10-27
tags:
  - m4a
  - acceptance
  - detail-panel
  - turn-cards
  - performance
  - agent-session-manager
---

# M4a 验收与收尾：回合卡详情面板逐条实测，脚本与文档收尾

## 协调者摘要（asm-m4a-accept）

- 结果：12 条验收标准 11 条通过，第 6 条部分通过（AUTO 卡显示原始 XML、compact 的展开链接折行，见"交给协调者" 1、2）。打开速度 release 70MB 首帧 45–79ms（页缓存热），达标
- 提交：三个截图脚本、Makefile 一行、spec / product-prd / PRD §3 / known-issues 和本总结同一个提交
- 测试：`cargo test --workspace` core 161、app 100 全过（2 ignored）；`make parity` 21/21（前两次各有 1 条因正在写入的 session 失败，第三次前台跑全过）
- 证据都在 `tmp/2026-10-27-m4a-turn-cards/accept/`：`detail*/`、`clicks/`（20 张）、`search/`（7 张）、`states-shots/`、`cold-scan/`、`thinking/`、`showall/`、`timing/`；截图全部用 Read 看过（裁出的详情面板或 2×2 拼图，`sheets/`）
- 测量：release 70MB 读取 20ms、转换 0.2ms、首帧 44.7 / 78.7 / 52.2ms；5MB 级 34.8–39.7ms；先逐出页缓存再打开，release 首帧 152 / 161 / 356ms。冷启动扫描（debug 7.9s）进行到 1815 / 2061 时点一行，首帧 31.7ms
- 脚本：`screenshot.sh`、`screenshot-search.sh`、`screenshot-clicks.sh` 加 `caffeinate -d -w`；`screenshot-clicks.sh` 删掉 Assistant chip，末尾再启动一次、预选 `5f6eea99`，截回合卡 5 张（共 20 张）
- 自己的决定：回合卡的点击放在 16–20 而不是第 11 张；第 10 条改用 cliclick（autoplay 等扫描结束才开始）；骨架、读取失败、空状态和 350 回合用 tmp 下合成的小数据目录
- 未验证：▲▼ 按钮没有用真实鼠标点（同一个 `step_hit` 由键盘测试和 autoplay 覆盖）；冷页缓存只在 `5f6eea99` 上量了
- 交给协调者：缺陷 2 条建议在 M4a 修，4 条可登记（见下文）；AGENTS.md 的截图命令说明要改（clicks 20 张）；known-issues 已删 3 条、加 2 条，验收发现的等你定

## 验收表

证据路径相对 `tmp/2026-10-27-m4a-turn-cards/accept/`。

| # | 标准 | 怎么验 | 结果 | 证据 |
|---|---|---|---|---|
| 1 | 顶栏写出回合数、AUTO 数、工具调用数，与卡片一致；没有开关栏 | 单元测试 `the_summary_counts…`；截图 | 通过（"13 turns · 4 auto · 481 tool calls"） | `detail/1-conversation.png` |
| 2 | 编号从 1 连续；AUTO 不占编号 | core 测试 `turns_start_at…`、`notifications_form…`；截图 | 通过 | `detail-run2/7-special.png`（AUTO 之后是 TURN 3）、`detail-350/8-big-session.png` |
| 3 | 折叠卡片的各项；超出部分能 Show all 展开 | 截图对照画稿 1；真实点击 Show all | 通过 | `detail/1-conversation.png`、`showall/2-show-all.png` |
| 4 | 悬停一格刻度改写统计行；点一格展开并滚到这一步 | `screenshot-clicks.sh` 的 cliclick 悬停和点击 | 通过（"35 / 120  Read  9-湖北…/10.jpg"；点后 TURN 1 展开、滚到第 35 步并展开） | `clicks/17-tick-hover.png`、`clicks/20-tick-click.png` |
| 5 | Show steps 后每步一行；工具行看 input / output；thinking 看全文；过程文字直接显示 | autoplay 截图；真实点击 Show steps、工具行、thinking 行 | 通过 | `detail/2…4-*.png`、`clicks/18-show-steps.png`、`clicks/19-tool-row-open.png`、`thinking/2-thinking-open.png` |
| 6 | 打断、API 错误、无回复、AUTO、compact 按画稿 4 | core 测试；三个 special session 截图 | **部分通过**：打断、API 错误、无回复、AUTO 的来源标签都对；AUTO 卡正文是原始 XML，compact 链接的 `▸` 折到第二行 | `detail/7-special.png`（API 错误）、`detail-run2/7-special.png`（打断）、`detail-run3/7-special.png`（compact）、`clicks/1-initial.png`（no reply） |
| 7 | 粘贴图片的 prompt 显示占位和大小 | core 测试；截图 | 通过（`image · 231 KB`） | `detail-run2/1-conversation.png` |
| 8 | 正则命中工具输出时停在那一步、已展开、高亮；`n` / `⇧N` / `⌘G` / `⇧⌘G` / ▲▼ 循环 | 步骤 4 的命中测试、键盘测试；autoplay `search-hit` / `next-hit` | 通过（▲▼ 未用真实鼠标点） | `detail/6-next-hit.png`、`cargo-test.log` |
| 9 | 超过 300 条消息的 session 能看到最后一个回合 | 本机索引里消息最多的 session 只有 186 条，另造 350 回合（700 条消息）的记录文件，借 `big-session` 步骤滚到底 | 通过（TURN 350） | `detail-350/8-big-session.png`、`detail/8-big-session.png` |
| 10 | 冷启动扫描进行中选中一行，对话立即出现 | 新 `ASM_DB` 冷启动，扫描中 cliclick 点第 2 行 | 通过：状态栏 "indexing 1815 / 2061 files" 时对话已在；日志 `detail:` 行在 `scan:` 之前 | `cold-scan/1-selected-during-scan.png`、`cold-scan/app.log` |
| 11 | 70MB 选中到对话出现 < 200ms（release） | `detail:` 日志行，见下节 | 通过（热 45–79ms）；冷缓存一般 150–160ms，偶发 356ms | `timing/` |
| 12 | 读取中显示骨架；失败时显示说明 | 合成数据：1GB→4GB 稀疏尾巴拖慢读取；`chmod 000` 制造读取失败；另看空 session | 通过 | `states-shots/1-skeleton-4.png`、`3-read-failed.png`、`4-no-messages.png` |

回归：`make screenshot-search` 7 个标记都到，7 张和 2026-10-23（M4 之前最后一次）对照，表格、Matches 列、片段高亮、状态栏一致，详情区换成回合卡（`sheets/search-*.png` 对 `sheets/search-m3ref-*.png`）。`make screenshot` 3 张正常。

## 实测数字

`detail:` 行的起点是 `select_row`，终点是带对话内容的首帧 paint。热缓存的几轮是 autoplay 在启动扫描结束之后选中（`timing.sh`），单位 ms。

| 构建 | session | 读取 | 转换 | 首帧 |
|---|---|---:|---:|---:|
| release | `5f6eea99` 66.7MB | 20.0–20.3 | 0.2 | 44.7 / 78.7 / 52.2 |
| release | `04f7d44c` 4.9MB | 12.8–18.3 | 0.7–1.0 | 39.0 / 39.7 / 34.8 |
| release，启动即选中（state.json 预选） | `5f6eea99` | 21.0–27.0 | 0.3 | 66.5 / 88.2 / 71.9 |
| release，先逐出页缓存 | `5f6eea99` | 335.1 / 126.9 / 119.1 | 0.3–0.5 | 355.6 / 152.0 / 161.0 |
| debug | `5f6eea99` | 20.3–22.2 | 0.7–0.8 | 98.8 / 101.2 / 51.2 |
| debug | `04f7d44c` | 14.0–15.2 | 3.3–4.0 | 42.3 / 48.6 / 38.5 |
| debug，先逐出页缓存 | `5f6eea99` | 119.2–123.8 | 0.9 | 150.9–154.5 |

- core 单独读取（`asm session --conversation`，release）：热 20.6ms，逐出后 107 / 110 / 229ms
- 逐出页缓存用 `evict.py`（只读映射加 `msync(MS_INVALIDATE)`，同 vmtouch -e；`mincore` 验证 100% → 0%）。本机这个文件读过一两分钟后再查已经是 0% 驻留：截图脚本里第二次启动预选它时，debug 读取 216–388ms、首帧 257–426ms 都是这种情况。所以"打开一个好一阵没看的大 session"基本是冷的
- 冷启动扫描（debug，新 `ASM_DB`）：2061 个文件、3.97GB，7.9s；首批行 3.2–3.5s 后上屏
- 骨架那次：4GB 稀疏尾巴的文件 debug 读取 1508ms

## 交给协调者

严重度：阻塞验收 / 应在 M4a 修 / 可登记。没有阻塞验收的。

1. **应在 M4a 修**：compact 分隔线的 `summary · 9.2k chars ▸` 折成两行，`▸` 单独掉到下一行（画稿 4 是一行）。每条分隔线都这样。证据 `detail-run3/7-special.png`、`cold-scan/1-selected-during-scan.png`。归 ui（`turn_cards.rs::render_compact` 的链接）
2. **应在 M4a 修**：task-notification 的 AUTO 卡正文是原始 XML（`<task-notification>` `<task-id>` `<tool-use-id>` `<output-file>`…），4 行折叠里只看得到 ID 和路径，有用的 `<summary>`（如 "Background command … completed (exit code 0)"）要点 Show all 才看到；画稿 4 是一句摘要，第 6 条验收写的是"按画稿 4"。本机这类 AUTO 卡 704 张。证据 `detail/7-special.png`、`detail-run2/7-special.png`。方向（推测）：读 AUTO 段时对 task-notification 取 `<summary>`（可带 status）作正文，原文留在 Show all；放 core 还是 app 转换由你定
3. **可登记**：冷页缓存下打开 70MB，release 首帧一般 152–161ms，偶发 356ms；本机文件很快被逐出页缓存，冷的才是常态（见"实测数字"）。方向（推测，未验证）：映射后 `madvise(MADV_SEQUENTIAL / MADV_WILLNEED)`，或改成大块顺序 `read`。和已有的"正则扫描冷缓存没量过"同类
4. **可登记**：整段显示时不去掉结尾空行。thinking 原文常以 `\n\n` 结尾，展开框下面空出两行（`thinking/2-thinking-open.png`，原文 `'…对齐方案的效果。\n\n'`）。`Folded.short` 只在折叠处去尾空行
5. **可登记**：步骤行的工具名列定宽，长名被截成 `ListAge…`，MCP 工具全是 `mcp__cu…`，分不出是哪个工具（`detail/2-steps-open.png`）；统计行同样被末尾省略（`mcp__cua-computer-use__get_window_state 8 · mc…`，`cold-scan/1-selected-during-scan.png`）
6. **可登记（与 M4a 无关）**：表格 Size 列 1000 MB 以上显示成 `024.2 MB`（列宽截掉首位，只在合成的 1GB / 4GB 文件上见到，真实最大 70MB）；prompt 和 Preview 原样显示 `<pasted_content id="9c86">` 标签
7. **只是提醒**：在 Metadata tab 上选中一行，`detail:` 的 first frame 要等切回 Conversation 才打点（见到过 1101ms），量速度时要停在 Conversation tab
8. **AGENTS.md**：`make screenshot-clicks` 现在 20 张，12–15 是自定义区间与日期选择器，16–20 是回合卡（脚本末尾第二次启动、预选 `5f6eea99`；换机器要换一个不再变化的 session 并重量坐标）；四个截图脚本都带 `caffeinate`。前面任务交的 `make screenshot-detail` 与 `ASM_DETAIL_AUTOPLAY` / `ASM_AUTOPLAY_SESSION|REGEX|SPECIAL|CUT_OUTPUT` 也还没写进去
9. **known-issues**：1–6 里你决定不在 M4a 修的，回复后我再登记

## 和 spec 不同的地方

- `screenshot-clicks.sh`：spec 写"第 11 张改为点第一张卡的 Show steps，并补一张点开工具行的截图"。第 10 张选的是"第 2 个项目的第 4 行"，每天都不一样，它的第一张卡可能没有步骤、prompt 行数也不定，坐标没法固定。改成脚本末尾再启动一次，state.json 只预选 `5f6eea99`（09-14 后不再变化），所有点击都在详情面板里，不碰表格。原来的 11–16 顺延为 11–15（Assistant chip 那张删掉）。按 Size 排序后点第一行的办法试过：排序要点表头的排序图标，排序后选中会跟着原来那个 session 滚走，第一行不在原位
- 验收标准 10：spec 写"用 detail autoplay 在扫描进行中选中一行"，但 autoplay 先等启动扫描结束才开始。没有改 app，用 `cold-scan.sh` 在扫描中真实点击一行（第一次点得太早，首批行 3.5s 后才上屏，表格还是空的；推迟到 3.5s 后重跑通过）
- 骨架、读取失败、空状态：spec 说"骨架靠人工看一次"。用 `states-prep.sh` 在 tmp 下造了一个小数据目录（独立 `ASM_DB` / `ASM_FAVORITES_DIR` / `ASM_TRASH_DIR`），state.json 预选，不用点击。FIFO 拖住读取的办法不行：发现阶段只收普通文件，FIFO 那个 session 直接变成 HIST

## 修改的文件

- `scripts/screenshot.sh`、`scripts/screenshot-search.sh`：app 存活期间 `caffeinate -d -w "$PID"`
- `scripts/screenshot-clicks.sh`：同样加 `caffeinate`；启动抽成 `launch`，可只预选一个 session；删 Assistant chip，编号顺延；末尾第二次启动截回合卡 16–20（新增 `hover`）；预选的 session 没打开时跳过并打印一句
- `Makefile`：`screenshot-clicks` 的帮助文字加 turn cards
- `kb/plans/2026-10-26-m4a-turn-cards-spec.md`：勾选步骤 1–6；文末"实施结果与差异"逐条列三份任务总结和本次与 spec 不同的地方
- `kb/docs/product-prd.md`：状态表的详情面板与搜索两行、§2 开头、§4.4 一句、§4.5 改写为回合卡的现状（M4b 待补充保留）、§6 加实测一列
- `kb/plans/2026-09-08-agent-session-manager-prd.md`：§3 M4 一行
- `kb/known-issues.md`：删 3 条，加"Prompts 列的数字比详情里的回合数多""Read 图片的步骤看不出读到了什么"

## 注意事项

- 构造详情的少见状态，用 tmp 下的小数据目录加 state.json 的 `selected_id` 最省事：启动即选中，不用点击也不碰真实索引。读取失败用 `chmod 000`（stat 还能成功，大小和 mtime 不变，启动扫描不动这一行）；拖慢读取用 `truncate -s 4G` 加一段没有换行的稀疏尾巴（不占磁盘，读取线程要扫过它，热缓存 debug 1.5s）。用完把文件截回原大小、恢复权限
- 要让 autoplay 的 `big-session` 步骤滚到某个合成 session 的底部，把它的文件名起成 `5f6eea99-…`（步骤按 id 前缀选）
- 量打开速度要分清页缓存冷热。本机 32GB 内存，70MB 的记录文件读过一两分钟就被逐出；`evict.py` 能精确制造冷的情形，`mincore` 能确认
- `make parity` 期间本会话自己的记录文件也在变（工具调用结果写进去），放后台跑时容易撞上；前台跑、跑的时候不发别的工具调用就稳定
- `screenshot-clicks.sh` 第二段的坐标：Show steps (1380, 345)、刻度第一行 y=363、第一条 Bash 行 (1100, 463)，都是 `5f6eea99` TURN 1 在默认窗口大小下的位置

## 遗留问题

- "交给协调者" 1–6 的处理等协调者决定
- 冷页缓存下的打开速度只量了 `5f6eea99`；release 首帧偶发超过 200ms
- tmp 下的验收目录约 800MB（主要是三份一次性索引），用完可删

## 已解决的已知问题

- **详情：索引扫描进行中选中一行，对话要等扫描结束才出现** — 读取线程独立于 store 线程（`2d8543b`）；冷启动扫描到 1815 / 2061 时实机点选，首帧 31.7ms（`cold-scan/`）
- **详情：Conversation 只显示索引里的前 300 条 user / assistant 文本，每条截到 4000 字** — 直接读记录文件、`list` 虚拟化的回合卡（`baaed71`）；700 条消息的合成 session 显示到 TURN 350，70MB 的 session 打开 45–79ms
- **搜索：工具调用与 thinking 里的命中在详情面板里看不到** — core 读取时标出所有命中位置（`200a567`），界面停到第一个并展开所在步骤；`detail/6-next-hit.png` 是 Bash 输出里的命中

## 相关文档

- [M4a spec](../plans/2026-10-26-m4a-turn-cards-spec.md) — 本次按"测试与验证"逐条验收，并勾选步骤、补"实施结果与差异"
- [M4 总 plan](../plans/2026-10-26-m4-detail-panel.md) — D4 打开速度口径、D7 Prompts 口径
- [core 步骤总结](2026-10-26-asm-m4a-core.md) — core 读取与真实数据核对
- [store 步骤总结](2026-10-26-asm-m4a-store.md) — 读取线程、转换与它的"遗留"
- [界面步骤总结](2026-10-27-asm-m4a-ui.md) — 界面实现、与画稿对照、返工 r1
- [产品 PRD](../docs/product-prd.md) — 更新状态表、§4.5、§6
- [原始 PRD](../plans/2026-09-08-agent-session-manager-prd.md) — 更新 §3 M4 一行
- [Known Issues](../known-issues.md) — 删 3 条、加 2 条
