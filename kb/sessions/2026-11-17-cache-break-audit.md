---
created: 2026-11-17
tags:
  - cache
  - audit
  - usage
  - ledger
  - agent-session-manager
---

# 缓存重建真实数据审计：冻结输入零差异

## 协调者摘要（cbreak-tracker-audit）

- 结果：3,398 个冻结前缀全部 SHA256 相符，133,186 次调用，842 条重建 TSV 与参考文件逐字节一致；Rust／参考脚本／口径未决差异均为 0，无生产代码修复。
- iterations：4 次误报全部消失，4 次后续重建全部补出为 compact；rewind 原 4 次不明变成 3 次不报、1 次换模型，另 4 次旧空闲变成 3 空闲、1 换模型。
- 不明：9 次，所在会话全部没有请求记录；332 个历史样本均仍超过原 TTL，最终为 314 空闲、5 换模型、5 登录标记、8 请求设置；14 次 TTL 转换及其 25 条后续重建逐条一致。
- 测试：冻结审计 1 passed；真实账本 core 聚合 1 passed；`make test` **670 passed / 11 ignored**（core 410 / 9、app 260 / 2），退出 0；parity 两轮分别 18/21、19/21，剩余均为动态会话变化，按 brief 决定 8 通过。
- 费用：隔离索引 `asm cost --compare` 成功，2198 个可比会话 $13656.87 vs $13930.33（−1.96%）；全部 3430 个会话 $14137.27。
- 账本：真实库只通过只读 SQLite backup 复制；副本重建修正 4 个响应，小时汇总逐字段相符，core 用量日 Cost 合计补计 **$0.3735984**，原 146246 条响应全部保留。
- 性能：三个最大会话热缓存中位数 23.325→24.073ms、21.856→22.385ms、19.530→20.079ms；增加 0.529–0.748ms，当前无需优化祖先回溯。
- 证据：`tmp/2026-11-16-cache-break-accuracy/cbreak-tracker/audit/`；提交见本提交，与审计测试同提交。
- 交给协调者：步骤 6 可据本报告签收；步骤 7 截图与共享文档收尾未做。332 表中 837afcce 的原请求间隔是文件顺序基准，Rust 正确使用祖先间隔，详见下文；不属于与新参考 TSV 的差异。

## 审计方法与可复跑测试

本次只新增测试和总结，没有修改 `crates/*/src`。新增 `tests/support/cache_audit.rs` 支持模块，`tests/conversation.rs` 追加两个 ignored 测试：

- `real_transcripts_cache_breaks`：只读取 manifest 指定的文件前缀，短文件或 SHA256 改变立即失败；一次仅保留一个冻结前缀到 scratch，文件名保持普通主 transcript 形态。用 `shasum -a 256` 校验，不额外引入依赖。读取完移除临时前缀。
- `real_usage_ledger_correction`：只打开明确指定的两个账本副本，走生产 `Ledger::usage` → `UsageData::totals`，验证 4 个会话的用量日增量，不启动 GUI。

第一项同时用参考任务保存的**调用输入事实**重放生产 CacheTracker：两条路径（真实 Reader 与输入事实重放）的完整 CacheBreak 向量必须相同，包括原因的附带值、回复时间、少读量、写入量、费用。这覆盖 Reader 的请求关联、祖先提取、去重、compact 生命周期与 iterations 口径。测试不读取参考 verdict；之后由 `compare.py` 将实际 TSV 与 `after.tsv` 比较全部 8 列及完整 session/message 键，检查键唯一性。

**诊断列的边界**：公开的 CacheBreak 只有 idle 原因含 gap/TTL，不保存非 idle 的间隔或 TTL。测试按冻结输入事实独立推导这两列，并要求每个 idle 的诊断值等于生产输出；非 idle 的这两列用于对照，不宣称是 Reader 暴露的字段。message_id 由已逐项相等的重放结果关联，未靠时间戳猜 ID。冻结事实不是参考判定结果，参考的全部原因仍由 Rust 实际运行产生。

复跑（从仓库根目录执行；所有路径可替换，但必须明确指定）：

```sh
ASM_CACHE_AUDIT_MANIFEST="$PWD/tmp/2026-11-16-cache-break-accuracy/cache-break-ref/manifest.json" \
ASM_CACHE_AUDIT_FACTS="$PWD/tmp/2026-11-16-cache-break-accuracy/cache-break-ref/calls.jsonl" \
ASM_CACHE_AUDIT_OUT="$PWD/tmp/2026-11-16-cache-break-accuracy/cbreak-tracker/audit/actual.tsv" \
ASM_CACHE_AUDIT_SCRATCH="$PWD/tmp/2026-11-16-cache-break-accuracy/cbreak-tracker/audit" \
cargo test -p agent-session-core --release --test conversation real_transcripts_cache_breaks -- --ignored --nocapture

uv run tmp/2026-11-16-cache-break-accuracy/cbreak-tracker/audit/compare.py

ASM_CACHE_LEDGER_BEFORE="$PWD/tmp/2026-11-16-cache-break-accuracy/cbreak-tracker/audit/ledger-before.sqlite" \
ASM_CACHE_LEDGER_AFTER="$PWD/tmp/2026-11-16-cache-break-accuracy/cbreak-tracker/audit/ledger-copy/usage-ledger.sqlite" \
ASM_CACHE_LEDGER_OUT="$PWD/tmp/2026-11-16-cache-break-accuracy/cbreak-tracker/audit/usage-deltas.json" \
cargo test -p agent-session-core --test conversation real_usage_ledger_correction -- --ignored --nocapture
```

输入沿用第 1 步的 60 天文件筛选及冻结 size，不重新按今天的日期筛选；文件内全部调用参与比较，不能再按回复时间截断。原始 `~/.claude` 只读，参考目录未写入。全量审计耗时 70.43 秒，包括 3398 次 SHA256 子进程与文件读写，不作为 Reader 性能数字。

## 逐条核对结果

`actual.tsv` 和 `cache-break-ref/after.tsv` 逐字节一致；`differences.json` 是空数组。原因计数：compact 276、idle 490、model 35、unknown 9、earlier_messages 2、system_or_tools 3、sign_in 11、request_settings 9、request_unchanged 7；effort / fast 为 0。

完整键与逐条解释在 `comparison.md`、`comparison.json`、`mixed-14.json`：

- **iterations 四例**：01dacff6、196366dd、603da7ec、d074913f 的零外层调用不再报失效；之后的 msg_011CggFmLC1bgxEwiPihSVQR、msg_011Ch4D4QdcNKiShb7qosX8E、msg_011Cgs6pqc9UjFRWFN9Gmx3F、msg_011Cghg4jJLfSA199AmHoyxQ 都为 compaction，少读分别为 336411、337990、339907、336014。
- **rewind 十个基准分叉**：原不明的 81956bd8、41140e46、4e5802b6 不再报；3c4e5381 归换模型。原空闲的 8016ab6c 归换模型；9dd59265、837afcce、da599fd6 仍空闲，少读和时间都取祖先。另 f8b16642、843dc5ae 新旧都不报。
- **9 个 unknown**：逐条检查对应会话的全部冻结输入事实，均完全没有请求记录；带请求记录的会话 unknown 为 0。
- **332 历史样本**：请求开始间隔都仍超过该历史脚本使用的 TTL，但按原因优先级只剩 314 idle，另外 5 model、5 sign_in、8 request_settings，符合 r2 报告。837afcce / msg_011CgR1X8nqJA2H8RZCUMPBZ 的历史请求间隔 82,209,474ms 使用文件上一调用，新祖先间隔为 83,832,058ms；仍超时。第一次核对脚本曾错误要求两者相等，改为明确记录基准变化，未修改生产代码或参考结果。
- **14 次 1h→5m**：读取量、前缀分段和该表列出的 25 条后续重建全部逐列一致。只有 7 次完整保留 1h 前缀；1c040d14 这次转换之后没有重建。2917171a 的一次丢失处仍属 1h 段，343189ms 不超时，归 request_unchanged；之后一次丢失 5m 段，373269ms 归 idle。具体每次 ID、gap、TTL 在比较报告中列出。

没有需要回到 Rust 修复的差异，因此没有新增失败复现测试或改动生产算法。

## 真实账本副本与用量日

通过 `sqlite3 "file:<真实库>?mode=ro" ".backup <scratch>/ledger-copy/usage-ledger.sqlite"` 取得一致副本，再 backup 成 `ledger-before.sqlite`。新版 asm 的 `ASM_DB` 和 `ASM_STATE` 始终位于 `ledger-copy/`，以新索引路径执行 `index --rebuild`；未打开默认 app / CLI 索引用于重建。

副本重建：3416 个文件、8.0G，得到 3430 个会话。原账本 146246 个响应主键全部保留，重建后 150885 个；其中活跃／此前未入账的其他响应属正常补入，不把它们的总额误算为本次 iterations 修复。四个目标会话的响应键集合都未变，且各自恰好只有目标响应一行变化。`ledger_check.py` 还从 responses 重新 GROUP BY 验证了目标小时内每个 model / fast / long_prompt / sub 分组，全部与 usage_hourly 相同。

| session | 修复前 input / read / output | 修复后 input / read / output | 既有 1h writes | 新加坡用量日 | Cost 增量 |
|---|---|---|---:|---|---:|
| 01dacff6 | 0 / 0 / 0 | 2 / 364422 / 223 | 1425 | 2026-11-03 | $0.0773524 |
| 196366dd | 0 / 0 / 0 | 2 / 364488 / 207 | 899 | 2026-11-15 | $0.0770456 |
| 603da7ec | 0 / 0 / 0 | 4 / 359673 / 304 | 9358 | 2026-11-09 | $0.0780306 |
| d074913f | 0 / 0 / 0 | 2 / 360509 / 3453 | 4941 | 2026-11-04 | $0.1411698 |

小时汇总的 token 增量逐字段恰好等于目标响应增量，response 计数不变。core 按 UTC+8、本地 4:00 分日聚合的 Cost 增量合计 **$0.3735984**，与 iter 已确认的费用修正完全一致。四个历史日期早于本次 Today（11-17），这四次修复本身不会改变 Today；活跃会话的新调用不在这项断言范围内。证据：`ledger-check.json`、`usage-deltas.json`、`usage-test.log`。

`asm cost --compare` 成功：2198 个有 CC 费用的会话，1432 个偏差在 1% 内、724 个在 1–10%、33 个低于 10% 以上、9 个高于 10% 以上；自算 $13656.87，CC $13930.33，−1.96%。本任务不处理既有的记录外调用差额。原始输出在 `cost-compare.log`。

## 读取耗时与祖先回溯开销

旧版用 `git archive f857f19` 导出到 `audit/before-src`，以绝对路径 `CARGO_TARGET_DIR=<scratch>/target-before` 单独构建 release asm；未切分支、未 stash。本 checkout 新版也为 release。按本机全部主 transcript 的文件大小排序取前三个，复制完整换行前缀到 `bench-claude`，两版都读取同一份副本。每个版本每个会话执行 3 次 `asm session-old ID --conversation`，交替运行，每次前完整读取文件预热，取日志 `read_ms` 的中位数；未 purge 缓存，没有把进程启动或输出耗时算进 read_ms。

| session | bytes | 旧版三次 ms | 新版三次 ms | 中位数旧→新 | 增量 |
|---|---:|---|---|---|---:|
| 4add415e（最大） | 76451511 | 24.066 / 23.088 / 23.325 | 24.671 / 24.073 / 23.792 | 23.325 → 24.073 | +0.748ms / 3.2% |
| 719f4771 | 73480131 | 22.341 / 21.856 / 21.694 | 22.385 / 22.372 / 22.438 | 21.856 → 22.385 | +0.529ms / 2.4% |
| bd5068d6 | 62382143 | 19.924 / 19.530 / 19.298 | 20.411 / 20.079 / 20.035 | 19.530 → 20.079 | +0.549ms / 2.8% |

机器 10 个逻辑 CPU，测量开始／结束 load average（1 / 5 / 15 分钟）均为 12.077 / 7.262 / 4.439；冻结审计当时也在运行，因此这是有背景负载的热缓存对照，不能外推冷缓存和 UI 首帧。相同输入、交替运行下开销不到 1ms，当前没有为了祖先回溯再优化的必要。证据为 `bench.json`、`bench-inputs.json`、18 份逐次日志及构建日志；没有单独量峰值内存。

## 回归与 parity

`make test` 退出 0：670 passed / 11 ignored，其中 core 410 passed / 9 ignored，app 260 passed / 2 ignored。相比 tracker 的 9 ignored 多出本次两个真实数据审计入口；它们已经分别显式执行通过。`git diff --check` 通过。唯一构建提示是现有依赖 block 0.1.6 的 future incompatibility。

两轮严格串行执行 `scripts/compare-with-python.sh ~/Library/Caches/cargo-target/release/asm <scratch>/parity-N`，明确设置各自的 ASM_DB / ASM_STATE；期间没有并行做其他工具工作。第一轮 18 PASS / 3 FAIL，第二轮 19 PASS / 2 FAIL，**脚本都退出 1**，不是退出 0 的全绿。5 组差异均只落在当时持续写入的会话，按 brief 决定 8 验收通过：

| 轮次 | 组 | 会话 | 差异 |
|---|---|---|---|
| 1 | list-code-30d | 3da4475a-2b50-4581-a70a-58d2f8332aff | 大小 2.1M→2.3M，更新时间 07:23→07:24 |
| 1 | list-code-30d | 39080042-1e5f-4091-b0c3-fedef76919a6 | 更新时间 07:23→07:24 |
| 1 | search-grafana | 3da4475a-2b50-4581-a70a-58d2f8332aff | 更新时间 07:24→07:25，匹配正文相同 |
| 1 | search-cjk | aadedae7-ecdc-4723-ba1c-33c94eb4d338、3da4475a-2b50-4581-a70a-58d2f8332aff | 两个结果块顺序交换；显示分钟均为 07:25，匹配正文不变 |
| 2 | list-code-30d | ee18a87a-5756-441a-99bf-c7d173614d4c | 更新时间 07:28→07:29 |
| 2 | list-code-30d | 39080042-1e5f-4091-b0c3-fedef76919a6 | 大小 2.2M→2.3M |
| 2 | search-cjk | ee18a87a-5756-441a-99bf-c7d173614d4c | 更新时间 07:29→07:30，匹配正文相同 |

`parity_check.py` 只移除列表元数据中的大小／更新时间、搜索头的更新时间，再比较全部非空行的多重集合，5 份 diff 都通过；除第一轮 search-cjk 的结果块移动，其余规范化后顺序也相同。没有静态文本、命中条数或匹配内容差异。证据：`parity-exits.txt`、两轮 log、各轮原始输出与 diff、`parity-analysis.json`。

## 交接与范围

没有运行 GUI、截图、全局安装、push 或 tag。截图为步骤 7。共享计划勾选、AGENTS.md、known-issues 与文档收尾由协调者处理。

并行 asm 文档任务的 AGENTS.md、Makefile、skills/asm/、kb/docs/app-architecture.md 和 asm session 总结均未改动或暂存；这些改动已由该任务提交为 `87cdcc3`，本提交顺接它，生产 crates 仍为审计时的 b408f4a。本提交只包含两个审计入口、支持模块和本总结。

## 相关文档

- [缓存准确性计划](../plans/2026-11-16-cache-break-accuracy.md) — 本次步骤 6 的依据。
- [tracker 实现总结](2026-11-17-cache-break-tracker.md) — 本次审计的 b408f4a 实现。
- [iter 实现总结](2026-11-17-cache-break-iter.md) — 四个响应的 token 与费用修正基线。
- [数据格式](../docs/claude-session-data-format.md) — 请求事实、用量、分段时长口径。
