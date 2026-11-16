---
created: 2026-11-17
tags:
  - cache
  - conversation
  - parser
  - agent-session-manager
---

# 按祖先调用与请求事实判定缓存重建

## 协调者摘要（cbreak-tracker-r1）

- 结果：审阅 R1 已修复。新增缓存辅助字段类型错误时只把该字段视为未知，不再丢掉合法 JSON 记录的正文、工具调用或导出；合法字段继续参与判定。
- 提交：见本提交；只改 `transcript.rs`、两个行为测试文件及本总结，不改索引解析、判定算法、渲染或其他任务的文档。
- 测试：先确认 2 个 Conversation 行为测试和 1 个导出行为测试失败，再实现；3 个新增测试通过。`make test` **673 passed / 11 ignored**（core 413 / 9、app 260 / 2），退出 0。
- 失败证据：`r1/red-tests.log`（2 failed，合法记录被计为 skipped）、`r1/red-export.log`（1 failed，两个 assistant 记录丢失）；通过证据 `r1/field-tests.log`、`r1/make-test.log`。
- 真实数据：重跑通过，3398 个冻结前缀均通过 SHA256 校验，842 条重建事件与参考 `cache-break-ref/after.tsv` 逐字节一致（0 diff）；证据 `r1/real-audit-retry.log`、`r1/verification.json`。第一次因原始文件被删除中止，不是解析／判定断言失败。
- 自己的决定：共用 `cache_field` 先借用完整 RawValue，再单独尝试目标类型转换；覆盖 parentUuid、requestRef、effort、请求 id／querySource／shapeHash／keep／tail／params，以及 params.betas。类型不符、越界、null 均等同缺失；不使用 tagged／untagged／flatten，也不把请求正文物化成 Value。
- 输入恢复：cc-renewer 的 `0a03c1ed-013c-4798-a51d-b83617e6f1b0.jsonl` 已被删；从既有 usage 快照取到同一 141549 字节前缀，SHA256 与冻结清单完全相同，只在 scratch 的清单替换此路径。未还原或修改 `~/.claude`。
- scratch：`tmp/2026-11-16-cache-break-accuracy/cbreak-tracker/r1/`；所有失败／通过日志、备用冻结前缀、重跑 manifest、输出 TSV 均在此处。
- 未做：GUI／截图、共享文档与计划收尾；不重复已签收的费用／账本／parity 全套审计。本次未改索引用量解析、搜索逻辑或导出格式。
- 交给协调者：R1 可按上述证据复核；未改 `skills/asm/references/cli.md` 或 `kb/plans/2026-11-15-asm-skill-and-cli/` 的并行改动。

### R1 实现与验证细节

`Option<T>` 原先仅兼容缺字段与 null，字段类型变化会使外层 Record 反序列化失败。新字段统一使用 `#[serde(default, deserialize_with = "cache_field")]`：先完整消费该值的原始 JSON，再尝试解析 T；失败只返回 None。因此 `params.betas` 含非字符串元素时，整份 beta 列表视为未知，外层请求的时间、shape 仍有效；params 本身不是对象时，仅 params 未知。真正语法错误的 JSON 仍报坏行，既有非缓存字段的解析策略不变。

新增测试覆盖：

- assistant 的 effort 对象、requestRef 数值及其他新增辅助字段类型错误：skipped_lines 为 0，过程正文、Read 工具与最终回复都保留，正常的模型变化仍被检测。
- 请求的 betas 对象／混合数组、非对象 params、字符串／越界 keep、非数组 tail：skipped_lines 为 0；有效开始时间仍给出 600000ms 空闲，有效 shape 仍能判 system/tools 变化，后续回复仍显示。
- 同一 malformed effort／requestRef 记录的完整导出和 `turns=1, tools=true` 导出均保留最终回复与工具信息，不再静默漏内容。

首次冻结重跑记录在 `r1/real-audit.log`，中止原因是 File::open NotFound。恢复前缀 SHA256 为 `3d9ecae47707e1f5d1126e5ed345e3672dd79b7a0ba663c106f8625665e3b439`。来源只读快照为 `tmp/2026-11-12-usage-stats/usage-p1-core/claude-snapshot/projects/-Users-reorx--local-share-cc-renewer-claude-cwd/0a03c1ed-013c-4798-a51d-b83617e6f1b0.jsonl`。重跑使用 `r1/manifest.json`，facts 仍为原 `cache-break-ref/calls.jsonl`；输出 `r1/actual.tsv`，日志 `r1/real-audit-retry.log`。没有删除缺失样本或重扫实时文件替代冻结输入。

## 协调者摘要（cbreak-tracker）

- 结果：步骤 3–5 完成；新增独立 `CacheTracker`，接入 Reader 的祖先链、请求事实与 `prompt_of`，补齐 app 新原因文案。
- 提交：见本提交；基于 `e0d9f82`，保持当前分支，不 push、不安装 CLI、不启动 GUI。
- 测试：`cargo test -p agent-session-core` 405 passed / 7 ignored；`cargo test -p agent-session-manager` 260 passed / 2 ignored；追加 5 个边界测试后的 `make test` **670 passed / 9 ignored**（core 410 / 7，app 260 / 2），全部退出 0。
- 红灯证据：`red-tracker.log` 5 failed、`red-reader.log` 7 failed、`red-app.log` 1 failed；旧四原因、TTL、去重与阈值的 2 个回归测试先通过，见 `legacy-tests.log`。
- scratch：`tmp/2026-11-16-cache-break-accuracy/cbreak-tracker/`，包含全部测试日志、只新增的 conversation diff 和原文件逐字节不变的断言结果；本任务不做耗时测量。
- 自己的决定：`RequestFacts` 的 shape / betas / sign_in / keep 保留 `Option`，请求时间也允许缺失；避免把缺失字段当成变化，与参考脚本一致。beta 按集合规范化再哈希。
- 未验证项：真实记录逐条比对、费用对比、parity、真实账本副本、读取耗时、GUI 截图均留给步骤 6、7；不据单元测试宣称 60 天计数已一致。
- 交给协调者：更新计划步骤 3–5、AGENTS.md 的缓存判定入口和规则；known-issues 的两条问题等 audit 签收后收尾。工作区另有 `kb/sessions/2026-11-17-asm-p1-accept.md`，未修改、未暂存。

## 概要

原实现以文件中的上一次调用为基准，把 rewind 放弃的后缀也当成丢失缓存；缓存时长只有一个标志，且不能说明请求层面的变化。本次把判定抽到不依赖记录格式的 `claude/cache.rs`，按接续的祖先调用比较，逐段保留 TTL，并按计划的优先级给出原因。Reader 负责拼输入，app 只负责文字。

compact 严格采用 `699c1a5` 后规则：命中缓存的调用不消耗标记，第一次真正重建才消耗；只有 compact 后已经有调用，新用户回合才清掉标记。手动 compact 后首条用户消息仍保留标记。

## 实现与修改的文件

- `crates/core/src/claude/cache.rs`：`CallInput`、`RequestFacts`、`CacheTracker`；去重、祖先回退、原因优先级、分段截断与合并、写入计价，13 个模块测试。
- `crates/core/src/claude/mod.rs`：公开 cache 模块，供后续 P2 复用。
- `crates/core/src/claude/conversation.rs`：替换 `CacheTrack/cache_check`；维护 uuid 父链、uuid 到调用映射、请求表和按来源的上次请求长度，回溯最多 10,000 步。每个响应分块的 uuid 都登记，重复响应随后直接返回，不重复回溯和解析用量。
- `crates/core/src/transcript.rs`：增加可选 `parentUuid`、`requestRef`、effort 和请求字段，tail 用 `IgnoredAny` 只计长度。
- `crates/core/src/claude/parse.rs`：移除 `prompt_of` 的临时 dead_code 标记，函数本身未改。
- `crates/core/src/model.rs`：6 个新原因变体和相关注释，`CacheBreak` 结构不变。
- `crates/core/tests/conversation.rs`：仅追加 8 个 Reader 行为测试；原来的 41 个测试逐字节保留。
- `crates/app/src/conversation.rs`：新原因的英文说明与 1 个测试（覆盖 8 条文字，包括 fast 开／关和两种请求设置原因）。
- `kb/docs/claude-session-data-format.md`：请求记录、关联字段、iterations 判定口径、祖先比较、原因、compact 和分段 TTL。
- `kb/docs/product-prd.md`：Conversation 的缓存重建说明。

## 测试与证据

所有命令都在当前 checkout 运行，首次构建前已 touch core / app 入口，避免共享 cargo target 复用其他源码树的陈旧产物。没有改 `classify`，没有打开默认索引或真实账本，没有运行真实数据命令。

测试顺序：先把旧判定逻辑放入独立 tracker 接口，用四原因、1h / 5m、去重、10k 阈值、费用的 2 个测试确认通过；添加新规则测试，得到 5 个断言失败；替换实现后 8 个测试通过。Reader 先追加内联记录测试，得到 7 个断言失败；接入后 49 个测试全过。app 先把新增原因暂按旧「cause unknown」输出，确认精确文案断言失败，再实现新文字。

| 日志 | 结果 |
|---|---|
| `legacy-tests.log` | 2 passed，旧行为基线 |
| `red-tracker.log` | 3 passed / 5 failed；rewind、TTL、请求时间、新原因、延迟 compact 均确实失败 |
| `tracker-tests.log` | 8 passed，核心新规则 |
| `red-reader.log` | 1 passed / 7 failed；非编译错误 |
| `reader-tests.log` | 49 passed，全部 conversation 行为测试 |
| `red-app.log` | 1 failed，实际 unknown，期望 effort 文案 |
| `core-tests.log` | 405 passed / 7 ignored，追加边界测试前 |
| `app-tests.log` | 260 passed / 2 ignored |
| `make-test.log` | 670 passed / 9 ignored；core 410 / 7、app 260 / 2；退出 0 |

后续追加的 5 个边界测试覆盖：分段截断、相邻合并、未知前缀回退；TTL 恰好相等、倒序及缺时间戳；compact 重置和重复调用不消耗标记；fast 倍率、最后 message prompt 的长上下文档位；缺请求字段与 fast 关闭。没有改费用表。

`git diff e0d9f82 -- crates/core/tests/conversation.rs` 只有末尾新增，归档为 `conversation-added-only.diff`。另直接比较 git 中原文件的全部字节与当前文件前缀，断言通过，见 `existing-tests-unchanged.txt`。

## 注意事项与取舍

- `RequestFacts` 与计划的非可选字段草案略有不同：缺失 shape / betas / keep 保留未知，缺请求开始时间保留 `None`。这是为了保持参考脚本的缺字段语义。两边有请求但无可用的具体变化时仍归 RequestUnchanged，不将缺字段本身当成变更；缺请求对象才回退 Unknown。
- beta 顺序和重复项不构成变化；`oauth-` 有无变化先于其他 beta 变化。shape 和 beta 哈希仅在同一 Reader 的内存里比较，本任务不持久化。
- `keep` 对比同一 querySource 的上次请求，而非上次有回复的调用；不同来源不相互污染。缺 querySource 使用独立的 `None` 桶。
- `CallState` 只保存后续比较所需字段与分段，不保留 CallInput 的 tokens、parent 和重复 msg_id；Reader 多出来的 uuid 表开销仍需 audit 实测。
- 空闲先尝试两端请求时间，任意一端缺失则整对退回回复时间；负间隔不转绝对值，TTL 等号不算超时。
- 说明行只陈述观察事实，RequestUnchanged 不推断服务端原因。输出格式、颜色、布局均未改。

## 遗留与交给协调者

本次实现已覆盖 rewind 误报、iterations 判定与真正重建的 compact 归因；真实 60 天的计数和四个会话的逐条结果尚未核对，留给 audit。按 brief 的文档归属，没有编辑 AGENTS.md、计划勾选、known-issues、next-up 或协调记录。

建议协调者将 AGENTS.md 中旧 `conversation.rs::cache_check` 改为 `claude/cache.rs::CacheTracker`，说明按祖先调用比较、请求事实归因及分段 TTL；在步骤 6 签收时更新两个已知问题的解决记录。无新增实现阻塞或需要用户决定的问题。

## 相关文档

- [缓存准确性计划](../plans/2026-11-16-cache-break-accuracy.md) — 步骤 3–5 的实现依据。
- [iter 总结](2026-11-17-cache-break-iter.md) — 本次接入的 prompt_of 与 schema 7 基线。
- [数据格式](../docs/claude-session-data-format.md) — 随实现更新的记录与判定口径。
- [产品说明](../docs/product-prd.md) — 随实现更新的用户可见行为。
- [M1 core 笔记](../notes/2026-09-17-m1-core-crate.md) — 解析器约束。
- [旧缓存说明行计划](../plans/2026-11-12-columns-and-cache-breaks.md) — 保留的阈值、展示格式和回归行为。
