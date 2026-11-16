---
created: 2026-11-15
tags:
  - cli
  - skill
  - search
  - cache
  - agent-session-manager
  - plan
---

# asm skill：教 agent 用 CLI 找会话、查缓存命中，并为此扩充 CLI

## PRD

### 背景

Agent Session Manager 的定位是本机 agent 会话的管理中心。除了人用 app，agent 也应该能用它：用户在 Claude Code 里说「帮我找一下近期关于某个 app 连不上网的问题解决会话」「看看最近的会话缓存命中得好不好」，agent 应该知道调哪个命令、搜什么、怎么判断结果。

现在的 `asm` 是给人看的命令行，输出和 Python 脚本逐字对齐，agent 用起来有这些问题（2026-11-14 在本机实测，脚本在 `tmp/2026-11-14-asm-skill-cli/`）：

- 搜索结果以消息为单位，不以会话为单位；几个词必须出现在同一条消息里；不能写「这几个词任一个」。
- 全文搜索只覆盖用户输入和 agent 回复。thinking 和工具的输入输出只能扫原始文件，30 天的数据（5 GB）一次要 6–8 秒。
- 输出给管道时仍带颜色码；没有 JSON；结果里没有标题、首条 prompt、工作目录，要再查一次。
- 会搜到正在进行的当前会话。
- 读一个会话只能整份导出，大会话导出一百多 KB。
- 看不到缓存命中率和缓存在哪里、为什么重建；有两种情况会让失效数字出错（`kb/known-issues.md`），由 [cache-break-accuracy](../2026-11-16-cache-break-accuracy.md) 修正。
- `asm` 不随 app 发布，只有开发机能用。

本机数据量：3,636 份记录文件、7.4 GB；最近 30 天里，工具的输入输出约 3.15 亿字符，用户输入 2450 万，agent 回复 490 万，thinking 80 万。thinking 块只有约 9% 有正文，其余只留了签名。

### 目标

做一个英文的 skill，名字叫 `asm`。agent 照着它用 `asm` 命令行完成两件事：

1. 按用户的描述找到过去的会话，核实内容后，给出会话、时间、项目、一句话结论和恢复会话的命令。
2. 检查一段时间内的缓存命中情况：命中率、重建了几次、为什么、花了多少，哪些会话最值得看，以及能怎么改。

### 共用概念

**文本类别**：一个会话里可搜索的文字分成以下几类，搜索、读取、输出都用这套名字：

| 名字 | 是什么 |
|---|---|
| `prompt` | 用户亲自输入的消息，每条开启一个回合，和详情面板里的回合编号同一口径 |
| `reply` | agent 写给用户的文字（包括过程中的说明，不只最后一条） |
| `thinking` | agent 的思考过程；只有约一成留有正文 |
| `tool` | 工具调用的输入（`tool-input`）和输出（`tool-output`） |
| `other` | 自动出现在用户一侧的文字：compact 摘要、后台任务通知、命令输出等 |

**会话级匹配**：搜索条件由几个「概念」组成，一个会话里要出现全部概念才算命中，各个概念可以出现在不同消息里。一个概念可以有几种说法（同义词、中英文、错误码），出现任一种就算。

**时间范围**：所有命令都用同一组参数表示时间范围：最近几天，或者起止日期。输出的第一行写明实际用了哪个范围，以及结果是否被截断。

**当前会话**：agent 在会话里调用 `asm` 时，默认不把这个会话本身算进结果，可以显式要求包含。

**输出**：默认输出简洁的文字，适合 agent 直接读，输出不在终端里时不带颜色。每个面向 agent 的命令都能改为输出 JSON，字段在 CLI 规范里写死。

**skill 的结构**：skill 放在本仓库，含一个总说明和三份参考文档：CLI 规范、找会话的方法、缓存检查的方法。CLI 规范先于实现写好，CLI 照着它实现；规范和实现对不上时测试会失败。

### 分期

| 期 | spec | 内容 |
|---|---|---|
| P1 | [session-search.md](session-search.md) | 找会话：新的搜索和会话命令、项目列表、按范围读会话、旧命令改名、通用约定；skill 的总说明、CLI 规范、找会话的方法 |
| P2 | [cache-health.md](cache-health.md) | 缓存检查：重建存进索引，总览和单个会话的重建明细；skill 的缓存检查方法。判定规则来自 [cache-break-accuracy](../2026-11-16-cache-break-accuracy.md)，它先做 |
| P3 | [pluggable-text-index.md](pluggable-text-index.md) | 搜索优化：可选的分词全文索引和向量检索，接在统一的接口后面 |
| P4 | [distribution.md](distribution.md) | 分发：`asm` 随 app 安装，skill 可以一键安装 |

P1 和 P2 期间，在本机通过 `make` 安装 CLI，在本仓库里使用 skill 验证；接口稳定后，P4 再对外发布。

### 各期都不做的事

- P1–P3 不改 app 界面；app 现有的搜索行为不变。
- 不搜 subagent 的对话，也不判断 subagent 的缓存失效（subagent 的 token 仍计入总量）。
- 不给搜索结果打相关度分数，P3 之前不排序以外的「智能」。
- 不把会话内容发到本机以外。

### 整体验收标准

- 在本仓库里新开一个 Claude Code 会话，问「帮我找一下近期关于某个 app 连不上网的问题解决会话」：agent 自己加载 skill，用 `asm` 找到相关会话（例子见 `skills/asm/references/finding-sessions.md` 例子 1，内容是虚构的：homehub 控制面板显示设备离线、fieldcam 录制任务的健康检查连不上摄像头网关），核实过内容，给出会话、时间、项目、一句话结论和恢复命令，没有把当前会话算进去。
- 同样新开会话，问「看看最近 7 天缓存命中得好不好」：agent 给出命中率、重建次数和原因、花费、最值得看的几个会话和建议；数字与 `asm cache` 的输出、app 表格的 Hit% 一致。
- `make parity` 照旧全部通过（旧命令改名后）。

## Tech Design

### 跨期决定

- **旧命令改名，新命令占用原名**：`search` → `search-old`、`session` → `session-old`，输出逐字不变，`scripts/compare-with-python.sh` 改调新名字；新的 `search` / `session` 是面向 agent 的会话级命令（用户 2026-11-15 决定）。被否决：保留旧名另起 `find` / `show`。
- **thinking 和 `other` 类别进全文索引**，工具输入输出仍靠扫描（数据量是对话文本的 10 倍以上，trigram 索引会到 GB 级），扫描加按字节预过滤（用户 2026-11-15 决定）。
- **缓存失效在建索引时算好存表**，查询不读原始文件（用户 2026-11-15 决定）。判定逻辑只有一份，app 回合卡和索引共用。
- **skill 名 `asm`，英文**，放在 `skills/asm/`；开发期间仓库里的 `.claude/skills/asm` 链接到它，本机 `make install-cli` 把 release `asm` 复制到 `~/.local/bin`（不用链接，原因见 [session-search.md](session-search.md)「脚本与构建」）。
- **`skills/asm/references/cli.md` 是 CLI 的规范**：每一期先改规范，再实现；core 的测试遍历 clap 命令树，规范里缺命令或参数就失败。
- **对 agent 输出透明的证据而不是分数**：每个结果给出各文本类别的命中数和片段，由 agent 判断相关性。

### 顺序与依赖

- P1 先做：通用约定（颜色、JSON、时间参数、排除当前会话、`--color`）在 P1 落地，P2 的 `asm cache` 直接沿用。
- P2 依赖 P1 的通用约定和 `cli.md`，以及 [cache-break-accuracy](../2026-11-16-cache-break-accuracy.md)（判定规则与 `CacheTracker`，用户 2026-11-16 决定先做）；两期都改 `schema.sql`，各自提升 `SCHEMA_VERSION`。
- P3 依赖 P1 的搜索接口：新后端只替换「找到哪些消息」这一步，输出不变。
- P4 最后：CLI 参数稳定后再打包发布。

### 实施步骤

- [x] P1 找会话，见 [session-search.md](session-search.md)（2026-11-17 完成）
- [ ] P2 缓存检查，见 [cache-health.md](cache-health.md)
- [ ] P3 可插拔文本索引，见 [pluggable-text-index.md](pluggable-text-index.md)（开工前先调研选型，补 Tech Design）
- [ ] P4 分发，见 [distribution.md](distribution.md)（开工前补 Tech Design）

### 各期都要更新的文档

- `skills/asm/references/cli.md`（先于实现）和对应的场景文档。
- `AGENTS.md`：「当前状态」加一行、「开发命令」里 `make asm` 的命令清单、`make parity` 的说明（改名）。
- `README.md` 的 CLI 一节（如果列了命令）。
- `kb/docs/product-prd.md` 的状态表；`kb/known-issues.md`（失效误判的两条由 cache-break-accuracy 删掉）。

### 跨期的风险

- 新命令要先增量扫描索引，和 app 写同一个索引时会争锁。P1 的处理：CLI 等锁 2 秒（app 的连接是默认的 5 秒），拿不到写锁就跳过扫描、用现有索引查询，并在 stderr 说明；新建或重建中的索引则等它建完（最多 60 秒）。开发期 CLI 默认用自己的 `index-cli.sqlite`（两边 schema 版本不同，共用会来回整库重建），只共用用量账本；P4 合并索引后争锁才常见，遗留的几处见 `kb/known-issues.md`。
- CLI 规范和 skill 写成英文，但用户的查询多是中文；场景文档里的例子要中英文都有，避免 agent 只扩写英文同义词。
