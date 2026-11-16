---
created: 2026-11-17
tags:
  - cache
  - usage
  - parser
  - ledger
  - agent-session-manager
---

# 按 iterations 核算用量，并验证索引升级后的账本修正

## 协调者摘要（cbreak-iter）

- 结果：步骤 2 完成；非空 iterations 全项计费，新增最后一个 message 的 `prompt_of`，索引版本 6 → 7；账本实现无需修改。
- 提交：见本提交；基于 `f857f19`，未改分支、未 push、未安装全局 CLI、未启动 GUI。
- 测试：`cargo test -p agent-session-core` **389 passed / 7 ignored**；`cargo test -p agent-session-manager` **259 passed / 2 ignored**，均退出 0。
- 失败证据：scratch 的 `red-parser.log`（实际 0 / 0 / 0，期望 2 / 223 / 364422）、`red-ledger.log`（v6 未触发重建）；完整通过输出为 `core-tests.log`、`app-tests.log`。
- 金额：相同真实记录输入冷建，全部会话 $14103.3059568 → $14103.6795552，**仅 4 次响应变化，增加 $0.3735984**；逐响应和实时新增调用的分解见 `cost-diff.json` / `cost-diff.md`。
- parity：两轮各 19 PASS、2 个动态差异；仅活跃会话大小 / mtime 和搜索结果排序变化，按 brief 决定 8 通过，具体会话见下文。
- scratch：`tmp/2026-11-16-cache-break-accuracy/cbreak-iter/`；所有真实数据命令使用此目录的索引与账本，未打开默认 app / CLI 索引或真实账本。
- 自己的决定：为排除其他会话持续写入，用旧索引的 `indexed_offset` 冻结 3744 份文件，再用旧 / 新二进制分别冷建；保留索引和清单，核对完成后清理 8.59 GB 文本副本。
- 未验证项：真实账本副本、GUI、缓存重建原因和读取耗时，均属后续任务；compaction 本机无样本，用 fixture 覆盖。
- 交给协调者：tracker 接入 `prompt_of` 时移除其临时 `#[allow(dead_code)]`；更新计划勾选及 AGENTS.md 账本解析升级规则；known-issues 的外层零用量问题已修，缓存判定部分仍待 tracker。没有发现他人未提交的文件。

## 实现与测试

`RawUsage.iterations` 使用 `Option<Vec<RawIteration>>`，`RawIteration` 的 type 和用量字段均可缺失，不使用 serde flatten / tagged 枚举。非空数组按所有项求和，包括 compaction；缺失、null 和空数组仍读外层。每项的 cache_creation 细分沿用原规则：有细分取细分，无细分把写入归到 5 分钟；负的 input / output 在每项内截为 0。web search 次数和 speed 仍取外层。

`prompt_of` 逆序找最后一个 message 项，返回输入总和与 cache read；不存在 message 项时用外层。它为下一任务准备接口，未接入现有 `cache_check`，也未改 Reader。索引里的 context 与长 prompt 分组继续使用计费输入总和，符合本阶段计划；现有 conversation 测试无需改变期望。

新增 6 个测试：3 个 parse 模块单元测试、2 个 parse_transcript 行为测试、1 个 ledger 升级行为测试。fixture 由 `uv run crates/core/tests/fixtures/gen_fixtures.py` 生成，独立放在 `tests/fixtures/iterations.jsonl`，不增加通用 discovery fixture 的会话数。

- 零外层响应：input / output / read 从 iterations 恢复，外层仍有的写入细分不重复计费。
- compaction 响应：外层只含 message 用量，额外 compaction 也计入 token、费用、context；覆盖单项缺写入细分与外层 web search 只计一次。
- 全量与逐行增量解析得到相同用量、费用和 context / peak。
- 最后一个 message 不一定是数组末项；判定 prompt 不用计费总和。
- 空 / null iterations、没有 message 项、负数用量的兼容路径。
- v6 索引里的旧零值已经写入账本；打开触发 schema 7 重建，保留已删 S1 的完整账本信息；扫描后 `iteration-0` 的 token 被覆盖，两个响应的小时汇总与响应数正确。

先运行行为测试，确认零用量和版本重建断言失败，再实现。首次测试还遇到测试代码的 SQLite `u64: FromSql` 编译错误，修成 i64 后保存了真正的断言失败日志；`red-tests.log` 是这份早期编译日志，不是红灯行为证据。账本的已有 `ON CONFLICT ... DO UPDATE` 与小时汇总机制满足要求，生产账本代码、结构和版本都未变。

修改文件：`claude/parse.rs`、`index/db.rs`、fixture 生成脚本及 `iterations.jsonl`、`tests/parse_transcript.rs`、`tests/ledger.rs`、数据格式文档和本总结。未改缓存判定、app 界面、共享协调记录和 AGENTS.md。

## 真实数据费用核对

改代码前先 touch core / app 入口，构建 HEAD 的 release asm，保存为 scratch 的 `asm-before`。使用新的 `before/index.sqlite` 路径执行 `asm cost --compare`，输出为 `cost-before.txt`。修改后新二进制在另一个新的 `after/index.sqlite` 冷建，输出为 `cost-after.txt`。这两次实时扫描之间增加了 52 次调用，增加费用 $3.2087626；已有响应只有下表 4 次改变。因此实时总差额 $3.5823610 = $3.2087626 + $0.3735984。

为使验收总额可以直接相减，`freeze_inputs.py` 从旧索引读取每个文件的 `indexed_offset`，只复制当时已经解析的完整前缀，保持 mtime，并复制 history。旧 / 新二进制分别以 `ASM_CLAUDE_DIR` 指向该副本，在 `frozen-before/`、`frozen-after/` 冷建。旧版冻结结果与第一次真实基线逐响应完全一致，费用总额也完全一致；新旧冻结结果主键集合相同，仅下表 4 行不同。`compare_costs.py` 对这些条件和合计差额作了断言。

所有四次模型都是 Opus 5.5；token 列顺序为 **input / cache write 5m / cache write 1h / cache read / output / web search**。

| session | msg_id | old tokens (in / 5m / 1h / read / out / web) | new tokens | old USD | new USD | delta USD |
|---|---|---|---|---:|---:|---:|
| 01dacff6-a59f-4b3c-8e6c-a0d8f093fad7 | msg_011CggFcrYPBek2yfyCyJZEM | (0, 0, 1425, 0, 0, 0) | (2, 0, 1425, 364422, 223, 0) | 0.011400000 | 0.088752400 | 0.077352400 |
| 196366dd-076b-4651-890b-767be310162a | msg_011Ch4Czde6AXH6cJVpS78Do | (0, 0, 899, 0, 0, 0) | (2, 0, 899, 364488, 207, 0) | 0.007192000 | 0.084237600 | 0.077045600 |
| 603da7ec-61df-4901-a1e1-ec268c219348 | msg_011Cgs6j9zHDKCsU7Cg8pLL8 | (0, 0, 9358, 0, 0, 0) | (4, 0, 9358, 359673, 304, 0) | 0.074864000 | 0.152894600 | 0.078030600 |
| d074913f-7c07-4462-b44f-d3b29c511d5e | msg_011CghftdmS9hCg9kK1NfxhG | (0, 0, 4941, 0, 0, 0) | (2, 0, 4941, 360509, 3453, 0) | 0.039528000 | 0.180697800 | 0.141169800 |

Total: 14103.305956800 -> 14103.679555200; delta 0.373598400

`cost-frozen-before.txt` / `cost-frozen-after.txt`：有 Claude Code 费用记录的 2188 个会话，自算合计 $13598.19338935 → $13598.56698775，同样增加 $0.3735984；CC 合计不变。全部 3423 个会话的合计如上表。CLI 显示两位小数，精确值直接从只读索引查询，不拿显示舍入值做逐条对账。

证据保留 `before/`、`after/`、`frozen-before/`、`frozen-after/` 的隔离索引与账本、`frozen-manifest.json`、脚本和输出；原始冻结文本副本在核对后清理。未读取或修改默认数据目录的任何库。真实账本副本上的重建验证仍留给 audit。

## 两轮 parity

使用 release 新二进制，顺序执行以下命令；期间未并行执行其他工具工作：

```sh
scripts/compare-with-python.sh ~/Library/Caches/cargo-target/release/asm tmp/2026-11-16-cache-break-accuracy/cbreak-iter/parity
scripts/compare-with-python.sh ~/Library/Caches/cargo-target/release/asm tmp/2026-11-16-cache-break-accuracy/cbreak-iter/parity-2
```

日志为 `parity-1.log`、`parity-2.log`，原始输出与 diff 在各轮目录。两轮脚本均因动态差异退出 1，不能称为退出 0 的全绿；按 brief 决定 8 的动态数据例外验收通过：

| 轮次 | 组 | 会话 | 差异 |
|---|---|---|---|
| 1 | list-code-30d | bcaa17a2-7507-4dee-bc04-66a9dc1da5ef | 文件 7.1M → 7.2M |
| 1 | search-cjk | fe446438-7eb4-4ad0-826a-88363ff9118f、13e33c17-52f9-4004-809c-f412360bc0e8 | mtime 23:57 → 00:57，整个结果块移到开头，命中正文相同 |
| 2 | list-code-30d | bcaa17a2-7507-4dee-bc04-66a9dc1da5ef | 文件 7.3M → 7.4M，mtime 01:00 → 01:02 |
| 2 | search-traceback | d380acf6-ac2e-4f61-89eb-e1ed562f8046 | mtime 01:03 → 01:04，结果块前移，命中正文相同 |

另外 19 组每轮逐字一致。专项断言移除列表动态元数据后完全相同、搜索输出去掉更新时间后整行多重集合完全相同，结果保存在 `parity-analysis.json`。没有静态内容或匹配语义差异。

## 遗留与交接

- `prompt_of` 只提供接口；最后一个 message 的缓存判定口径将在 tracker 接入，现在原 `cache_check` 仍消费 `tokens_of`，本机 4 次单项明细的 token 已随之修正。
- schema 7 首次打开会重建索引并更新有记录文件的账本响应；不要用本阶段二进制打开默认真实索引。审阅和后续开发继续按 brief 使用隔离路径。
- 共享计划、AGENTS.md、known-issues 由协调者维护，本提交只更新直接相关的数据格式文档；本任务没有新增已知缺陷。

## 相关文档

- [缓存失效准确性计划](../plans/2026-11-16-cache-break-accuracy.md) — 实施步骤 2 的依据。
- [数据格式](../docs/claude-session-data-format.md) — 更新 iterations 计费、判定接口及账本升级口径。
- [M1 core 笔记](../notes/2026-09-17-m1-core-crate.md) — 解析器与增量语义约束。
- [用量账本计划](../plans/2026-11-16-usage-stats/usage-ledger.md) — salvage、覆盖和小时汇总机制。
- [官方 compaction 文档副本](../refs/claude-api-compaction-threshold.md) — compaction 明细计费的依据。
