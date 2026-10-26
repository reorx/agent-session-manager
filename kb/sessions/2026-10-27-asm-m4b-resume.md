---
created: 2026-10-27
tags:
  - asm
  - m4b
  - claude-code
  - resume
---

# M4b 步骤 0：Claude Code 恢复命令的只读调研

## 协调者摘要（asm-m4b-resume）

- Q1【确定，文档＋静态代码】：本机 Claude Code **2.1.310** 按完整 ID 先查当前项目和 worktree，再查其余项目；全局有多个有效副本时不任选一个，而是报找不到。证据 §1、§2 E3。
- Q2【确定支持，未实跑】：原项目目录消失不妨碍查找保留的 transcript；可从存在的目录执行 `claude --resume <id>`。普通恢复不自动重建项目；工作目录与 worktree 特例见 §3。
- Q3【确定支持，写入风险为推测】：`--resume /absolute/path/<id>.jsonl` 可读取收藏；相对路径不走文件分支。普通恢复可能直接写回收藏目录，不能把能读取等同于可直接开放按钮。证据 §2 E2/E4、§4。
- Q4【确定】：`-r` 是 `--resume` 别名，`--continue` 选当前目录最近会话，不能替代指定 ID；空格、中文可用单引号，路径含单引号须转义，不能把 `~` 放进单引号期待展开。见 §5。
- Q5【建议】：目录在且原记录在 → `cd '<绝对项目路径>' && claude --resume <完整id>`；目录不在且原记录在 → `claude --resume <完整id>`；仅收藏副本 → 本期先置灰，修改“Claude Code can no longer resume it”的错误提示。见 §6。
- 测试 `the_resume_command_without_a_project_folder_follows_step_0` 期望 `Some("claude --resume <id>")`；保留 `a_missing_project_folder_disables_only_open_folder`（正常 transcript 行）。见 §6。
- 版本边界：官方说明跨项目 ID 查找自 **2.1.223** 起可用；旧版不能套用目录缺失的结论。CLI 帮助未写文件参数，但官方文档与本机代码均支持。见 §1。
- 未验证：真实交互恢复、收藏写回、fork 后快照不变；必要实验列在 §7，由协调者取得用户授权后安排，本任务没有执行。
- 交付：仅本总结及 `tmp/2026-10-27-asm-m4b-resume/` 证据；不提交、不改代码/plan/AGENTS、不构建、不启动 app、不运行任何 resume/continue 命令。已有 `AGENTS.md` 修改未触碰。

## 概要

M4b spec 的两项风险均以“只按当前项目目录找记录”为前提。本机版本已支持跨项目 ID 查找和绝对 JSONL 路径加载，因此 D8 的目录缺失回退命令可保留。收藏的技术能力判断需要修正，但是否开放“从快照继续”还涉及写入位置与新旧 session ID 的产品语义，本轮只提出建议，不修改 spec。

使用 kb 的总结格式；按 reverse-engineer-anything 的证据分级原则进行静态检查。当前没有 REA MCP，也没有 `rea` CLI，因此以下是普通 shell/二进制内嵌 JavaScript 的观察，不是 REA Evidence 或实际运行观测。官方网页通过 firecrawl-search / firecrawl-scrape 获取；不安装工具、不修改配置。

## 1. 官方资料与本机版本

2026-10-27 本机只运行了 `claude --version`、`claude --help`。`which claude` 为 `/Users/reorx/.local/bin/claude`，符号链接指向 `/Users/reorx/.local/share/claude/versions/2.1.310`，`file` 识别为 Mach-O arm64。帮助确认 `-r, --resume [value]`、`-c, --continue`、`--fork-session` 的语义。

官方页面：

- [Manage sessions — Resume by session ID or name](https://code.claude.com/docs/en/sessions#resume-by-session-id-or-name)：ID 可跨目录；查找顺序是当前项目/worktree，再全机其他项目；其他项目仅一个包含消息的副本时才命中。
- [Manage sessions — Sessions in other worktrees and projects](https://code.claude.com/docs/en/sessions#sessions-in-other-worktrees-and-projects)：picker 遇到已消失的项目目录，会在当前目录恢复。这是 picker 的明确行为描述，不应把 picker 的复制 `cd` 操作当作 CLI ID 模式的行为。
- [CLI reference — CLI flags](https://code.claude.com/docs/en/cli-reference#cli-flags)：明确允许绝对 `.jsonl` 路径，并说明 **v2.1.223 之前** ID 搜索只有当前项目及 worktree。
- [Manage sessions — Where transcripts are stored](https://code.claude.com/docs/en/sessions#where-transcripts-are-stored)：默认存储布局及 `CLAUDE_CONFIG_DIR`；超长编码路径会截断并附哈希。
- [Manage sessions — Branch a session](https://code.claude.com/docs/en/sessions#branch-a-session)：`--fork-session` 创建新 ID，普通并行恢复则会共同写同一 transcript。

本地证据目录 `tmp/2026-10-27-asm-m4b-resume/`：`claude-help.txt`、`claude-version.txt`、`search.json`、`sessions.md`、`cli-reference.md`、`inspect_binary.py`、`binary-evidence.txt`。网页为查询时快照，不保证将来的官网与此相同。`search.json` 没有返回 search id，未发送无法关联的搜索反馈。

## 2. 可复查的本机静态证据

二进制 SHA-256：`ffb64331bc0ee1b69efb7c2c428bd3ab516bc31b75bf60886b356c97c1c25ffd`。

内嵌代码被压缩且不同 chunk 会重复短函数名，下面提供**原二进制从 0 开始的字节偏移**及精确搜索锚点，比整个二进制的行号更可靠。临时提取脚本只读取二进制、写入 tmp 证据，不执行提取的 JavaScript：

```sh
uv run --no-project tmp/2026-10-27-asm-m4b-resume/inspect_binary.py
```

不依赖临时文件也可复查，例如：

```sh
rg -a -o 'function KFe\(e\).{0,100}' /Users/reorx/.local/share/claude/versions/2.1.310
rg -a -o 'async function mYr\(e,n=.{0,540}' /Users/reorx/.local/share/claude/versions/2.1.310
rg -a -o 'async function vJo\(e,n\).{0,510}' /Users/reorx/.local/share/claude/versions/2.1.310
```

| 证据 | 偏移 / 提取文件位置 | 观察与含义 |
|---|---|---|
| E1 参数与编码 | 216820199、216827540；`binary-evidence.txt:8`、`:5` | `le` 从 Node `path.isAbsolute` 导入；`KFe(e)` 为 `le(e)&&e.endsWith(".jsonl")`。必须绝对路径且以小写 `.jsonl` 结尾 |
| E2 CLI 文件分支 | 236014679、225599750；`:11`、`:14` | `if(KFe(e.resume))` → `I7e(e.resume,P)` → `hMe(...)` → `bMn(...)`。`I7e` 调 `Lne` 读取传入文件并验证消息/对话链，无“只能位于 projects 下”的普通本地文件路径限制 |
| E3 CLI ID 查找 | 226971476、226976411、216829979；`:17`、`:20`、`:23` | `hMe` 的字符串分支为 `QMe(...) ?? vJo(...) ?? EJo(...)`；分别为本项目加载、worktree 回退、`mYr` 全 projects 扫描。`mYr` 遍历项目目录，第二个含消息副本出现就返回 null；不是全盘扫描 |
| E3 存储根目录 | 216832450、216831632；`:26`、`:29` | `im()` 为配置目录下 `projects`；路径编码使用 `replace(/[^a-zA-Z0-9]/g,"-")`，长路径另有截断哈希逻辑 |
| E4 恢复持久化位置 | 234536449、215673081；`:32`、`:35` | `bMn` 非 fork 时调用 `_b(id,"resume",dirname(transcriptPath))`，`_b` 把这个目录写入 session identity 的 projectDir |
| E4 输出路径 | 215674772、221235995；`:41`、`:48` | `Lbe()` 读取上述 projectDir；`V8()` 优先用它，`_p()` 拼 `<id>.jsonl`。因此正常恢复收藏会把输出路径指向收藏目录，存在修改快照的具体代码依据 |
| E5 普通 cwd 恢复 | 234469107；`:44` | 本构建 `oio(e,o)` 返回 `{pin:void 0,switched:!1}`，没有基于普通 transcript 的 projectPath 强制切换目录。`bMn` 仍有 worktree 恢复分支；未完整证明所有特殊 session 的 cwd 行为 |

读取 `~/.claude/projects/` 的目录名确认有 `-Users-reorx-Code-agent-session-manager`。没有扫描真实对话正文。编码是查找用的内部 key，不可反向推断真实路径：空格、中文、斜杠、连字符等都可能映射成 `-`，存在碰撞；命令必须用记录中的真实绝对项目路径。

## 3. 项目目录不在时怎么恢复

当原始 JSONL 仍在当前 Claude 配置根目录的 `projects/` 中，完整 ID 可以从另一个存在的目录查到，不要求原项目目录还存在。默认本机版本无需 `CLAUDE_CONFIG_DIR`，也无需重建空目录。没有命中或全局出现歧义时会报找不到 ID，而不是自动选一个。

对普通本地 session，静态代码支持继续使用启动目录；所以省略 `cd` 的含义是“在用户终端当前目录恢复对话”，不是还原已删除的项目文件。原会话属于 worktree、已 `/cd` 迁移、正在后台运行时还有专门逻辑，不能承诺所有恢复都保持当前目录。目录在时仍建议显式 `cd`：既保留 D8 意图，也让用户在正确工作区继续。

`CLAUDE_CONFIG_DIR=/some/root` 改变的是 Claude 的配置/状态根，而不是工作目录；查找对应 `/some/root/projects`。它可以让 Claude 找到另一套存储，但不会自动把任意 JSONL 目录当成 project，也不会恢复代码文件。不要为了恢复收藏把它直接设为收藏目录：这会同时让 Claude 在其中读写配置、会话等状态。

顺带发现 ASM 支持 `ASM_CLAUDE_DIR`：如果应用读取的根与用户终端 Claude 使用的根不同，单独复制 ID 命令仍可能失败。这不是本轮新增功能；交协调者评估后续命令是否需要显示存储根或使用明确文件路径，本轮不扩大 `resume_command` 的接口。

## 4. 收藏副本：可加载，但普通恢复可能破坏快照

文件参数的能力是确定的：例如 `claude --resume '/absolute/favorites/claude/projects/<key>/<id>.jsonl'` 进入文件加载分支。不要用相对路径替代，后者可能按名称搜索。无消息、没有有效对话链、文件不可读仍会失败；“路径受支持”不等于任何文件都能恢复。

ASM 的实际布局比任务中的简写多一层：`crates/core/src/favorites.rs:22` 是 `<favorites-root>/claude/projects/<project>/<id>.jsonl`，`:40`–`:42` 复制主记录及同名资源目录。它符合可加载 JSONL 的形式；不能继续声称 Claude Code 完全无法恢复这种副本。

然而 E4 显示，非 fork 恢复会采用传入文件的目录作为持久化目录，并复用 ID。**静态推测：用户继续聊天会修改收藏快照，而不是自动把它导入默认 projects。** 未实际发起恢复，故不把最终磁盘副作用写成实测事实。

可考虑的后续方案是 `claude --resume '<收藏绝对路径>' --fork-session`：新 ID 明确表达“从收藏分支继续”，静态 `bMn` fork 分支调用 `NWe(...)`，不采用源目录。但仍应验证新 transcript 的落点、资源引用与快照 hash 不变，再决定按钮名称和是否支持。不要在 Copy resume 内悄悄变成新 ID；另一条“Restore a copy”操作会更容易解释。

## 5. Shell 引用与参数选择

面向 macOS zsh/bash，统一对绝对路径做 POSIX 单引号引用即可保留空格、中文、`$`、反引号、括号等字符。路径内部的单引号替换为 `'\''`（结束引号、转义单引号、重新开引号）。例如：

```sh
cd '/Users/reorx/Code/中文 项目' && claude --resume 11111111-1111-4111-8111-111111111111
cd '/Users/reorx/Code/O'\''Brien 项目' && claude --resume 11111111-1111-4111-8111-111111111111
```

使用真实绝对路径，不能生成 `cd '~/Code/项目'`，单引号内 `~` 不会展开；也不要把路径压缩成 projects 的编码 key。`&&` 保证 `cd` 失败时不在错误目录启动。可选地用 `cd -- '<path>'`；绝对路径本身不以 `-` 开头，当前 D8 格式已经足够。

完整合法 UUID 可以直接放在命令中。若以后接收任意来源的 ID/名称，也应作为一个 shell 参数引用或严格验证 UUID。`-r` 和 `--resume` 没有功能差异，保留长选项便于阅读；`--continue` 是“最近会话”语义，不能替代用户选中的 session，也不应以短 ID 前缀代替完整 ID。

## 6. 给实现者的行为与测试建议

以下区分纯命令函数与按钮可用性；`resume_command` 无法仅凭三个参数判断 History/收藏，须由 `available` 处理。

| 情况 | M4b 建议 | 测试期望 |
|---|---|---|
| 原记录在，项目目录在 | `cd '<绝对项目路径>' && claude --resume <id>` | 原有目录/空格测试继续成立；普通无特殊字符路径是否带引号可沿用现有风格 |
| 原记录在，项目目录不在 | `claude --resume <id>`，Copy resume 可用，Open folder 置灰 | `resume_command(id, "/missing/project", false) == Some(format!("claude --resume {id}"))` |
| 只剩收藏，目录在或不在 | 本期 Copy resume 先置灰，导出/Reveal 等保持原规则 | 保留 `a_favorite_whose_original_is_gone_cannot_be_resumed_but_can_be_exported` 的可用性断言，但说明这是本期保护快照的选择，不是 CLI 能力限制 |
| 仅 History、没有 transcript | Copy resume 置灰 | 原有 History 测试不变 |

建议收藏提示改为 **“Only the favorite copy is left; restoring from a favorite is not supported here yet”**，不再声称 Claude Code 不支持。该提示与暂缓启用是本次建议，尚未修改 spec 或取得新的产品决策。

补充后续 BDD：路径含单引号、中文加空格、字面量 `$`/反引号、绝对路径不使用 `~`；原目录缺失与收藏缺原件同时成立时，收藏规则优先，不能因能生成裸 ID 命令就误启用。当前步骤不写测试或代码。

## 7. 未执行的实验与交给协调者的事项

核心查找结论已有文档和静态代码，不需要为推进正常 Copy resume 而强行运行实际 session。收藏功能若要本期启用，需协调者先问用户，再用**一次性复制件**实测；以下只是实验设计，未创建 fixtures、未复制真实 session、未运行命令：

1. 选择用户批准的已结束 session，复制完整 JSONL 与同名资源目录到 `tmp/<date>-resume-probe/source/<id>.jsonl`，不要选另一个 agent 正在验收或仍运行的会话。复制件的 `cwd` 指向一个不存在目录，保留消息链/ID，并记录文件 hash。
2. 跨项目测试：将复制件放在隔离根 `<probe>/config/projects/<encoded-missing-dir>/<id>.jsonl`，在 `<probe>/work` 下执行 `CLAUDE_CONFIG_DIR='<probe>/config' claude --resume <id>`。观察是否加载对话、界面 cwd、错误文字；隔离根可能要求单独认证，不能把认证失败当作查找失败。退出后检查隔离根新增文件。测试前由协调者决定隔离认证方法，不自动复制凭据。
3. 文件测试：在 `<probe>/work` 下执行 `CLAUDE_CONFIG_DIR='<probe>/config' claude --resume '<probe>/source/<id>.jsonl'`，观察加载；如获准发送一条无工具的简短消息，再检查 source hash 和写入路径。不要对真正的 favorites 文件执行。
4. fork 测试：恢复一份全新的 source 复制件，执行 `CLAUDE_CONFIG_DIR='<probe>/config' claude --resume '<probe>/source/<id>.jsonl' --fork-session`。观察新 ID、生成 transcript 的目录、source 主文件和同名资源目录是否保持不变。预期新 ID 写在隔离 config 的当前项目存储中，源快照不变；这是待验证期望。
5. 如需验证歧义，再在隔离 config 的两个非当前项目目录中各放相同 ID 的有效 transcript，执行同一 ID 命令，预期 not-found；移除一份后预期可加载。不要在真实 `~/.claude/projects` 制造副本。

协调者需同步：M4b 风险 1 可以关闭并保留 D8；风险 2 改为“支持文件输入，但需决定收藏恢复/fork 与保护快照”；评估 >=2.1.223 的版本前提；收藏测试命名与提示不应被误读成技术上的永久限制。未改 `kb/known-issues.md`、`kb/todo.md`、`kb/next-up.md` 等协调者独占文件。

## 修改的文件与验证边界

新增本总结；辅助抓取与静态提取结果在 `tmp/2026-10-27-asm-m4b-resume/`（gitignore）。没有修改应用代码、spec 或共享协调记录，没有提交。本轮未启动浏览器、模拟器、桌面 app，也没有创建需要清理的测试进程；不执行会影响另一个验收 agent 的全局 cleanup。

## 相关文档

- [M4 总计划](../plans/2026-10-26-m4-detail-panel.md) — D8 是本次验证的命令规则。
- [M4b spec](../plans/2026-10-26-m4b-actions-and-extras-spec.md) — 步骤 0、Copy resume 可用性及待定测试的依据。
- [收藏计划](../plans/2026-10-26-favorites.md) — 快照语义及此前“恢复到 ~/.claude”的待验证设想。
