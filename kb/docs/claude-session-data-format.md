---
created: 2026-09-08
updated: 2026-10-27
tags:
  - claude-code
  - data-format
  - jsonl
  - research
---

# Claude Code Session 数据格式调研

本文记录 `~/.claude` 下与 session 相关的所有数据源、JSONL 记录类型和字段，是数据层（索引、解析、派生字段）设计的依据。数据来自 2026-09-08 对本机 654 份顶层 transcript（148,556 行、845MB）的全量扫描，Claude Code 版本主要为 2.1.216 ~ 2.1.260。

> 参考实现：`~/Code/scripts/claude_session_search.py`（Python CLI，已实现搜索 + `--list`），本项目是它的 GUI 化与索引化升级。

## 1. 数据源总览

| 路径 | 内容 | 生命周期 |
|---|---|---|
| `~/.claude/projects/<encoded-path>/<session-uuid>.jsonl` | 主 transcript，每行一条 JSON 记录 | 会被 `cleanupPeriodDays`（默认 30 天）清理 |
| `~/.claude/projects/<encoded-path>/<session-uuid>/subagents/agent-<id>.jsonl` + `.meta.json` | 子 agent（Agent tool）的 transcript，与主 session 同格式；meta 含 `agentType`、`description`、`toolUseId`、`spawnDepth` | 随主 session |
| `~/.claude/projects/<encoded-path>/<session-uuid>/tool-results/*.txt` | 超长 tool 输出的外置文件 | 随主 session |
| `~/.claude/projects/<encoded-path>/sessions-index.json` | Claude Code 自己维护的 session 索引（version 1），仅部分项目存在（本机 85 个项目里 15 个，共 170 条） | 不可靠，只能当补充 |
| `~/.claude/projects/<encoded-path>/memory/` | 该项目的 auto-memory | 与 session 无关 |
| `~/.claude/history.jsonl` | 全局提示历史：每条 `display`、`project`、`sessionId`、`timestamp`(ms)、`pastedContents` | **永不清理**，是 transcript 被清理后唯一能恢复 session 存在性的来源 |
| `~/.claude/sessions/<pid>.json` | 当前存活进程的运行时信息 | 进程退出即删除，可用于判断"session 正在运行" |
| `~/.claude/file-history/` | 文件修改备份，与 `file-history-*` 记录对应 | — |

规模（本机，2026-10-23）：156 个项目目录、1,942 份主 transcript（2.9GB）+ 257 份 subagent transcript（196MB）；最大单文件 70MB；history.jsonl 已被 Claude Code 截短到 408 行。2026-09-08 时是 654 份、917MB，增长很快，估算性能时以最新数字为准。Rust 索引器冷启动全量扫描约 3.6s（页缓存热），其中写库约 3.3s。

### 项目路径编码

`encode(path) = re.sub(r'[^A-Za-z0-9-]', '-', path)`，例如 `/Users/reorx/Code/scripts` → `-Users-reorx-Code-scripts`。编码不可逆（`/` 和 `_`、`.` 都变成 `-`），所以**真实项目路径必须从记录里的 `cwd` 字段或 history.jsonl 的 `project` 字段取**，目录名只用于匹配。

同一个仓库的 worktree 会形成独立"项目"（如 `-Users-reorx--herdr-worktrees-tenderbuddy-hhsw-e-ui`），UI 上可考虑按 git 仓库聚合。

## 2. Transcript 记录类型

按本机出现次数排序。`type` 字段是判别键。

### 2.1 对话记录（有 `message` 字段）

| type | 次数 | 说明 |
|---|---|---|
| `assistant` | 51,506 | 模型输出。`message.content` 是 block 数组 |
| `user` | 31,369 | 人类输入 **或** tool_result 回传（两者共用 type=user） |

公共字段（user/assistant/system 都有）：`uuid`、`parentUuid`（构成消息树，null 为根）、`isSidechain`、`timestamp`(ISO)、`sessionId`、`cwd`、`gitBranch`、`version`、`userType`(=external)、`entrypoint`(cli / sdk-cli)。

user 特有：`promptId`、`toolUseResult`(tool_result 记录才有)、`sourceToolAssistantUUID`、`isMeta`（harness 注入的内容，如 skill 正文，非人类输入）、`promptSource`(typed / sdk / system / queued)、`origin`(`{kind: human | task-notification | auto-continuation | peer}`)、`permissionMode`、`slug`、`forkedFrom`、`isCompactSummary`、`imagePasteIds`、`sessionKind`(=bg 时为后台 session)。

assistant 特有：`requestId`、`effort`、`message.model`、`message.id`、`message.usage`、`attributionSkill` / `attributionPlugin` / `attributionMcpServer`（标记该消息由哪个 skill/plugin 驱动）、`error` / `isApiErrorMessage`、`isAbortedMidStream`。

`message.content` 形态：

| 形态 | 次数 | 说明 |
|---|---|---|
| user: string | 1,824 | 纯文本人类输入（最常见的"用户消息"） |
| user: `text` block | 719 | 带附件或 harness 注入（isMeta）时用数组 |
| user: `image` block | 32 | 图片粘贴 |
| user: `tool_result` block | 28,831 | 工具返回；`content` 为 string 或 `[{type:text}]`；`tool_use_id` 关联 |
| assistant: `tool_use` block | 28,831 | `name`、`input`(dict)、`id` |
| assistant: `thinking` block | 15,878 | `thinking` 文本（本机大量为空串 + `signature`） |
| assistant: `text` block | 6,800 | 正文 |

**判定"人类输入的消息"**（沿用 `human_prompt_text`）：`type == user` 且非 `isSidechain`，content 为 string 或含 text block 且**不含 tool_result**，且非 `isMeta`，且文本不以 `[Image: source:` 开头。再用 `clean_prompt` 去掉 `<system-reminder>`、`<command-message>`、`<ide_opened_file>`、`<ide_selection>` 包裹块，并把 `<command-name>/<command-args>` 还原为 `/cmd args`。

### 2.2 Session 元数据记录（无 `message`，直接可用作表格列）

| type | 出现于多少 session | 字段 | 用途 |
|---|---|---|---|
| `last-prompt` | 585 / 654 | `lastPrompt`（截断到 ~200 字，末尾 `…`）、`leafUuid` | "最后一条用户消息"的快速来源；文件里可能多条，取最后一条 |
| `cost-state` | 1,636 份文件（2026-10-23） | `totalCostUSD`、`totalAPIDuration`、`totalToolDuration`、`totalLinesAdded/Removed`、`totalDuration`、`startTime`(ms)、`modelUsage{model: {inputTokens, outputTokens, thinkingTokens, cacheRead.., cacheCreation.., costUSD}}` | 费用/耗时列；多条取最后 |
| `ai-title` | 5 | `aiTitle` | AI 生成的短标题（新版本才有） |
| `custom-title` | 2 | `customTitle` | 用户 `/rename` 的标题 |
| `agent-name` | 5 | `agentName` | 同 custom-title 语义 |
| `pr-link` | 1 | `prNumber`、`prUrl`、`prRepository` | 关联 PR |
| `frame-link` | 1 | `frameUrl`、`title`、`path`、`artifactCount` | 关联 Artifact |
| `summary` | 0（本机无） | `summary`、`leafUuid` | 旧版本 `/compact` 摘要；其他工具（ccusage 等）依赖它，需兼容 |
| `mode` / `permission-mode` / `atis-latch` | 每 session 多条 | `mode`、`permissionMode` | 权限模式（bypassPermissions / auto / default / plan）可做过滤器 |

标题优先级建议：`custom-title` > `agent-name` > `ai-title` > `sessions-index.json.summary` > 第一条人类消息首行。

### 2.3 其他记录（索引时可整行跳过）

| type | 次数 | 说明 |
|---|---|---|
| `attachment` | 27,175 | harness 注入（`attachment.type`: total_tokens_reminder、hook_success、skill_listing、edited_text_file、file …），**体积大且无检索价值**，但 `attachment.type == file` 时含 `@file` 引用的文件路径可做"引用文件"派生字段 |
| `system` | 2,965 | `subtype`: turn_duration(1267)、stop_hook_summary(1245)、away_summary、local_command、api_error、compact_boundary、model_refusal_fallback …；`turn_duration.durationMs` 可累计 |
| `file-history-snapshot` / `file-history-delta` | 4,417 | 文件备份跟踪；`trackingPath` 可派生"本 session 修改过的文件列表" |
| `queue-operation` | 1,478 | 排队输入（`operation: enqueue/dequeue`, `content`） |

### 2.4 用量与费用

费用和 context 占用都从 assistant 记录的 `message.usage` 算，结论来自 2026-10-23 的全量统计：

- **一次 API 响应会写成多条记录。** 每个内容块（thinking、text、tool_use）各一条 assistant 记录，它们的 `message.id`、`requestId` 相同，`usage` 完全相同。本机 117,536 条 assistant 记录只对应 53,525 次响应，不去重 token 会多算约 2.2 倍。去重键依次取 `message.id` → `requestId` → 记录 `uuid`。增量索引可能把同一次响应的几条记录切在两次扫描之间，所以去重要跨扫描进行。
- **`usage` 的字段**：`input_tokens`（未命中缓存的输入）、`cache_creation_input_tokens` 及其细分 `cache_creation.ephemeral_5m_input_tokens` / `ephemeral_1h_input_tokens`、`cache_read_input_tokens`、`output_tokens`（含 thinking）、`server_tool_use.web_search_requests`、`speed`（`standard` / `fast`）、`service_tier`、`inference_geo`。没有细分字段的旧记录，缓存写入全按 5 分钟算。本机 `speed: fast` 有 422 条，`web_search_requests` 全为 0，`service_tier` 全为 standard，`inference_geo` 只有 `global` 和 `not_available`（没有要加价 1.1 倍的 US-only）。
- **`<synthetic>` 不是 API 调用**：`message.model == "<synthetic>"` 的记录是 Claude Code 自己写的（中断、报错），usage 全 0，不计价，也不算进模型列表。
- **subagent 的花费不能忽略**：257 份 subagent transcript 有 5,545 次响应，cache read 6.16 亿 token、cache write 3,157 万 token，折合几百美元。它们的记录 `isSidechain = true`；主 transcript 里没有 sidechain 的 assistant 记录。
- **`[1m]` 标记不可靠**：`message.model` 从不带 `[1m]`，`cost-state.modelUsage` 的键有时带（`claude-opus-5[1m]`）。带不带标记都有 session 超过 200K（最高 827K），所以 context 窗口按模型查表（Haiku 4.5 为 200K，当前其余模型 1M），不看标记。
- **`cost-state` 不适合当费用来源**：1,636 份文件里有，其中 670 份 `totalCostUSD = 0`，更早的 session 没有这条记录；被 resume 过的 session 常常只记了最后一次运行（例：一个 session 的记录只覆盖了 32M cache read 里的 3.1M）。它确实包含 subagent 的费用，而且单次运行内的计算口径与官方价格一致（1 小时缓存写入按 2 倍输入价，两份抽样精确到小数点后 6 位）。按 usage 自算后，在 973 个 `cost-state` 非 0 的 session 上：61% 误差在 1% 以内、96% 在 10% 以内，合计相差 1.07%（自算偏低）。偏低推测来自 transcript 之外的调用（标题生成、auto mode 分类器、compact 等），未验证。

## 3. 派生字段（每个 session 一行）

| 字段 | 来源 | 备注 |
|---|---|---|
| `session_id` | 文件名 stem / `sessionId` | |
| `project_path` | 第一条记录的 `cwd`；history 的 `project` | 目录名不可逆解码 |
| `project_key` | 目录名 | |
| `git_branch` | 首条/末条记录的 `gitBranch` | 可能中途切换，存首末两个 |
| `title` | 见 2.2 优先级 | |
| `first_prompt` | 第一条人类消息（clean 后） | |
| `first_prompt_at` (= `started`) | 该消息 `timestamp` | history 模式取 group 首条 `timestamp` |
| `last_prompt` | `last-prompt` 记录或最后一条人类消息 | |
| `last_message` | 最后一条 assistant text block | "展示最后一条消息"模式 |
| `updated` | 文件 mtime 或最后记录 `timestamp` | 排序默认键 |
| `prompt_count` | 人类消息数 | |
| `message_count` | user+assistant 记录数 | |
| `tool_use_count` / `tool_names` | tool_use block 聚合 | 可做 "用过 Bash/Edit" 过滤 |
| `size_bytes` | 文件大小 | |
| `cost_usd` | assistant 记录的 `message.usage` × 价格表，subagent 计入父 session | 见 2.4；`cost-state.totalCostUSD` 另存作对照 |
| `context_tokens` / `context_peak` | 最后一次 / 最大一次主线调用的输入 token（含缓存读写） | 除以模型窗口得到占用比例 |
| `models` | `message.model` | 不含 `<synthetic>` |
| `duration_ms` | `cost-state.totalDuration` 或 `turn_duration` 累计 | |
| `permission_mode` | 最后一条 `permission-mode` | |
| `is_background` | `sessionKind == bg` | |
| `entrypoint` | cli / sdk-cli | SDK 驱动的 session 可默认隐藏 |
| `subagent_count` | `<sid>/subagents/*.jsonl` 数量 | |
| `source` | transcript / history | history-only session 缺大部分字段 |
| `is_live` | `~/.claude/sessions/*.json` 或文件 mtime < N 秒 | |
| `pr_url` / `artifact_urls` | pr-link / frame-link | |
| `files_touched` | file-history-delta.trackingPath 去重 | |

## 4. history.jsonl 分组规则

- 每行一个提示：`{display, pastedContents, timestamp(ms), project, sessionId?}`；`sessionId` 从 2025-11 起才有（本机 11,335 / 11,520 行有）。
- 按 `encode(project)` 分组；有 `sessionId` 的按 id 聚合；没有的按时间间隔 > 2h 切分为匿名 session。
- transcript 存在时以 transcript 为准；仅当 transcript 缺失时用 history 补位并标记 `source = history`。
- history 的 `display` 里 `/` 开头的是斜杠命令，取第一条非命令行作为 first_prompt；`[Pasted text #1 +5 lines]` 是粘贴占位符，正文在 `pastedContents`。
- core 的 `history_sessions` 与 `history_prompts` 共用 `group_history`，详情保留组内每行（含斜杠命令和空文本），条数与表格 Prompts 一致。粘贴正文先从内联内容或 `paste-cache` 还原，再经 `clean_prompt` 清理；无法还原时保留占位符。`conversation_from_history` 将每条 prompt 转成一个 `NoReply` 回合，并设置 `Conversation.history_only`，供界面区分历史提示与普通的缺失回复；History 没有 transcript 字节数，`bytes` 为 0。

## 5. 增量索引要点

- transcript 只追加不修改（compact 也是追加 `compact_boundary`），因此可按 `(path, size, mtime, indexed_offset)` 记录已索引字节偏移，只解析新增行。
- 单个 session 文件内 `last-prompt` / `cost-state` 会反复追加，派生字段取最后一次出现即可，增量时覆盖。
- 文件被清理（删除）时：保留 DB 中的元数据行，标记 `source = history`，消息全文索引删除。
- `~/.claude/projects` 用 FSEvents（`notify` crate）监听，防抖 500ms–1s；live session 每次 turn 都会追加，UI 可显示"运行中"。

## 6. 其他 agent 的 session（扩展方向）

本机 `~/.codex/sessions/<year>/...rollout-*.jsonl` 有 144 份 Codex CLI transcript；旧项目 `~/Code/agent-sessions` 已有把 Claude Code 与 Codex 都转成统一中间格式（`SessionMeta{title, description, agent, source}` + `Message{role, content}`）的 Python parser，可作为第二数据源的 adapter 参考。本项目一期只做 Claude Code，但数据模型里预留 `agent` 字段。


## 收藏副本

收藏保存的是操作当时的快照，包含主 transcript、subagents、meta 和 tool-results。原会话继续写入时，收藏内容不自动刷新；原会话被 Claude Code 清理后，收藏仍可显示、搜索和导出。取消收藏后恢复读取原会话；原会话也不存在时，只保留索引中的历史信息和费用。

默认目录是索引旁的 `favorites/`，布局为 `claude/projects/<project_key>/<id>.jsonl` 和同名目录。`ASM_FAVORITES_DIR` 优先于索引旁 `config.json` 的 `favorites_dir`，两者都没有时使用默认路径。CLI 和桌面应用共用配置。

发现文件时按 session id 选择收藏副本，主文件及 subagent 都来自同一侧。只有主 JSONL 已落位的收藏才有效，孤立子目录和 `.staging/` 不参与覆盖。复制保留文件修改时间，并截去 JSONL 未完成的末行。

取消收藏移到系统废纸篓；测试使用 `ASM_TRASH_DIR` 隔离。更换目录迁移文件，目标已有同 id 时保留目标并留下旧副本；嵌套目录和 Claude 数据目录不允许作为迁移目标。详见 [收藏方案](../plans/2026-10-26-favorites.md)。
