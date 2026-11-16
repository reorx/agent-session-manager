---
created: 2026-11-16
tags:
  - usage
  - ledger
  - core
  - sqlite
  - testing
---

# 用量统计第 1 期 core：独立账本、重建前抢救与按小时计价

## 协调者摘要（usage-p1-core）

- 结果：完成 ledger 步骤 1、3–6；不改 app、不打开真实索引/账本、不执行真实数据导入。
- 提交：代码、测试、spec、data-layer-design 与本总结在同一个提交；hash 见交付回复，或 `git log -1 --format=%H -- kb/sessions/2026-11-16-usage-p1-core.md`。
- 测试：core 365 通过 / 0 失败 / 5 忽略；app 151 / 0 / 2；真实数据快照 parity 21 / 0（两次活数据运行受并发写影响）；输出统一在 `tmp/2026-11-12-usage-stats/usage-p1-core/`。
- 实测：回填 356.324 ms；完整账本 17,485,824 字节；有用量的索引重开核对 2.145 ms；129,372 次调用 / 2,225 个有用量 session / 5,510 桶。
- 版本 4 原样副本的抢救＋重建 1,347.400 ms；保留全部调用及 3,361 个 transcript session 的元数据。
- 决定：本地墙上时间减 4 小时；恢复失败不得静默禁用账本；来源索引与账本错误分开标记；小时汇总单独重算也有事务。
- spec 修正：Today 调用 totals 时上界必须取下一个 UTC 整点，否则会漏掉当前小时；core 半开区间契约未改。
- 差异：HEAD 为 bd56682、SCHEMA_VERSION 为 6，保持不变；原始备份实际为版本 4。
- 未验证：GUI 实机、真实数据导入和生产账本，均属于后续任务。
- 交给协调者：下面列出 app 接口、Today 上界要求及 AGENTS/known-issues 的接续建议。

## 概要与实现

账本与索引分文件、分连接，app 和 CLI 的同目录索引共享 `usage-ledger.sqlite`。扫描期间先积累 LedgerBatch，索引事务结束前提交账本，再提交索引。删 transcript、删 subagent、改写文件、清空索引都不删除账本调用。相同调用 upsert 更新 token，时间取最早值，小时汇总按 UTC 小时与定价键重算，费用不入账本。

打开时用两边的小汇总表比较调用数，仅回填缺少的 session；索引重建前从旧 api_usage 全量抢救，旧表/列缺失可跳过。账本只在版本 0 建表，后续结构变化必须加迁移；更新版本的账本主动禁用，不改文件。损坏文件连同 WAL/SHM 改名留存并从索引回填；来源索引的 SQLite 错误保持原身份，账本错误不会落入删除索引的分支。恢复失败后数据库对象继续报错，必须重新打开，避免下一次扫描静默漏记。

没有改解析、价格表、索引 schema、SCHEMA_VERSION 或 app crate；chrono-tz 只作为测试依赖。新模块、类型与接入点见本提交中的 `ledger.rs`、`ledger.sql`、`usage.rs`、`model.rs`、`claude/paths.rs`、`index/db.rs`。

## 接口

下个 app 任务可直接使用：

```rust
// agent_session_core::claude::paths
pub fn ledger_path(index: &Path) -> PathBuf;

// agent_session_core::index::Db
pub fn ledger_usage(&self, ids: Option<&[String]>)
    -> anyhow::Result<Option<Vec<SessionUsage>>>;
pub fn salvage_into_ledger(&self, source: &rusqlite::Connection)
    -> anyhow::Result<()>;

// agent_session_core::usage
pub const DAY_START_HOUR: u32 = 4;
pub fn usage_day<Tz: chrono::TimeZone>(ts_ms: i64, tz: &Tz)
    -> chrono::NaiveDate;
pub fn day_start_ms<Tz: chrono::TimeZone>(day: chrono::NaiveDate, tz: &Tz)
    -> i64;

#[derive(Clone, Debug, Default)]
pub struct UsageData {
    pub sessions: BTreeMap<String, SessionUsage>,
}
impl UsageData {
    pub fn replace_all(&mut self, sessions: Vec<SessionUsage>);
    pub fn apply(&mut self, sessions: Vec<SessionUsage>);
    pub fn totals(&self, from_ms: i64, until_ms: i64) -> Totals;
    pub fn totals_with(&self, from_ms: i64, until_ms: i64,
        price: impl Fn(&str) -> Option<ModelPrice>) -> Totals;
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    pub responses: u64,
    pub tokens: Tokens,
    pub cost: CostLines,
    pub unpriced_responses: u64,
}
```

`HourlyUsage`、`LedgerSession`、`SessionUsage` 在 `agent_session_core::model`，字段与 spec 相同，额外都实现 Clone/Debug/Default/PartialEq。费用总额取 `totals.cost.total()`，In 取 `totals.tokens.input_total()`。UsageData 是带公开 sessions 字段的结构体，不是 BTreeMap 的类型别名；replace_all/apply 逐 session 替换，不合并小时桶。账本读出的列表按 session id 排序，小时按完整键排序；无 session 元数据的调用会返回空元数据，无调用的已存 session 会返回空 hours。

`ledger_usage(None)` 取全量，`Some(&ids)` 只返回命中的 session，`Some(&[])` 返回空列表。最外层 None 表示只读连接或新版账本主动禁用；故障是 Err，不能当成没有用量。app 的现有错误链处理可以接住 Err。

`totals` 按 `[from_ms / 3_600_000, until_ms / 3_600_000)` 取桶，严格遵循 spec。**Today 的调用示例已经在 spec 修正**：`from = day_start_ms(usage_day(now, &Local), &Local)`，`until = (now / 3_600_000 + 1) * 3_600_000`。直接传 now 会排除当前小时。整点日期区间的上界保持不变；半小时时区按 plan 允许的整桶近似处理。

`day_start_ms` 遇到二义时间取较早瞬间；不存在时逐秒前进到下一个存在的本地时间。usage_day 在 naive_local 上减四小时，夏令时切换日 04:00 也正确。

账本底层接口（app 无须直接使用）：

```rust
pub const LEDGER_VERSION: i32 = 1;
impl Ledger {
    pub fn open(path: &Path) -> anyhow::Result<Self>;
    pub fn open_in_memory() -> anyhow::Result<Self>;
    pub fn write(&self, batch: &LedgerBatch, index: &Connection) -> anyhow::Result<()>;
    pub fn refresh_hourly(&self, session_id: &str) -> anyhow::Result<()>;
    pub fn response_counts(&self) -> anyhow::Result<HashMap<String, u64>>;
    pub fn usage(&self, ids: Option<&[String]>) -> anyhow::Result<Vec<SessionUsage>>;
}
```

`Ledger::open` 遇到新版本返回可 downcast 的 `NewerVersion(pub i32)`，由 Db 转成主动禁用。LedgerBatch 的 responses/projects/sessions 与 spec 相同，另有 Default 和 is_empty。salvage_into_ledger 公开是给 usage-p1-import 使用；本任务没有运行真实导入。

## 测试与验证

| 验证 | 通过 | 失败 | 忽略 | 证据（任务 scratch 下） |
|---|---:|---:|---:|---|
| cargo test -p agent-session-core | 365 | 0 | 5 | core-verified.log |
| cargo test -p agent-session-manager | 151 | 0 | 2 | app-verified.log |
| 完整 parity（相同真实数据快照） | 21 | 0 | — | parity-snapshot.log、parity-snapshot/ |

行为测试先写，最初因接口不存在编译失败；独立小时重算、无调用元数据读取、恢复失败保护各自有断言失败的红灯日志。账本测试 27 项通过、测量测试默认忽略 1 项；usage 测试 9 项全部通过。core 的其他忽略项来自已有测试，测量测试另外显式运行通过。

**parity 共调用 5 次（4 次有效完整比较，1 次包装脚本配置错误）**：

1. 活数据：17 组通过、4 组失败（parity.log）。probe 和其他项目的会话持续写入导致文件大小、更新时间、排序或扫描文件数变化；匹配正文未见差异。
2. 活数据重跑：19 组通过、2 组失败（parity-rerun.log）。probe 文件仍在增长；另一个项目的新会话在 Python 和 asm 两次读取之间新增了一条命中。
3. 为避免外部写入，APFS clone 3,734 个真实数据文件到 claude-snapshot；首次快照运行的 Python 包装脚本错误地拦截了 Makefile 查询构建目录的 `python3 -c`，asm 路径变为不存在的 `/release/asm`，这一轮不是有效比较（parity-snapshot-setup.log）。修正包装器只拦截参考脚本，并用 make -n 核对了真实可执行路径。
4. 正确的快照运行：20 组通过、1 组失败（parity-snapshot-cutoff.log、snapshot-cutoff.diff）。只有 search-bash 的扫描数相差 1，467 个命中完全一致：文件 `9b888fe7-90fa-4d3d-a5cd-58f6b4f1d10d.jsonl` 的 mtime 为 2026-09-17 12:08:56；Python 在 11-16 12:08:32 起算最近 60 天，asm 在 12:08:59 起算，它刚好跨过截止线。
5. 同一份快照再完整重跑：**21 组通过、0 失败**（parity-snapshot.log）。未修改数据、mtime、查询参数或比较规则。

运行入口仍是 make 的 parity 目标；scratch 中的 parity.mk / parity-snapshot.mk 只覆盖证据输出目录。快照运行通过 ASM_CLAUDE_DIR 指向克隆数据，Python 由 reference-snapshot.py 加载原脚本，仅覆盖 PROJECTS_DIR / HISTORY_FILE / PASTE_CACHE_DIR 三个输入常量。未改原参考脚本、仓库比较脚本、HOME 或真实数据。克隆脚本、包装器及日志均留在任务 scratch 供复核。最终通过证明固定真实数据上的行为一致；不宣称两次活数据比较通过。

### 与 spec 的用例对照

`crates/core/tests/ledger.rs` 中以下 16 个名字与 spec 完全一致，覆盖验收标准 1–8、10、14：

- a_scan_writes_responses_sessions_projects_and_hourly_rows_to_the_ledger
- a_response_cut_between_two_scans_is_in_the_ledger_once
- deleting_transcripts_then_scanning_and_rebuilding_leaves_the_ledger_unchanged
- a_subagent_deleted_on_its_own_leaves_the_index_but_stays_in_the_ledger
- a_rewritten_transcript_does_not_remove_ledger_rows
- an_index_of_another_version_is_copied_into_the_ledger_before_it_is_rebuilt
- an_old_index_without_usage_tables_is_rebuilt_without_failing
- a_corrupt_index_is_deleted_and_the_ledger_is_left_alone
- a_corrupt_ledger_is_renamed_aside_and_refilled_from_the_index
- rows_the_ledger_lacks_are_copied_from_the_index_on_open
- the_first_open_backfills_sessions_whose_transcripts_are_gone
- a_ledger_of_a_newer_version_is_left_alone_and_the_index_still_works
- two_indexes_in_one_directory_share_the_ledger_without_double_counting
- a_failed_scan_leaves_nothing_in_the_ledger
- session_info_survives_a_rebuild_that_brings_the_session_back_from_history
- the_ledger_schema_is_what_its_migrations_build

同文件额外覆盖：unchecked 事务的 autocommit 标记、upsert 最早时间和全部分组键、写失败回滚并清 pending、打开时账本错误不删索引、损坏表恢复、双索引同时初始化、独立小时重算原子性、写入中损坏恢复未提交用量、账本先提交/索引提交失败后的幂等重试、无调用的 session 元数据、恢复失败后的持续错误。另有默认 ignore 的 measure_backfill_and_reopen；显式传本任务目录下的副本路径才运行。

`crates/core/tests/usage.rs` 的 spec 8 项也使用原名：the_usage_day_starts_at_four、a_day_starts_at_four_local_in_whole_hour_and_half_hour_zones、days_around_a_dst_change_are_23_and_25_hours_long、totals_cover_the_hours_in_range_and_nothing_else、costs_follow_the_price_table、a_model_without_a_price_costs_nothing_and_is_counted_as_unpriced、hourly_costs_add_up_to_the_per_response_costs、applying_changed_sessions_replaces_only_those_sessions。另加 Samoa 跳过整天时下一个存在瞬间的测试。

## 实测与证据

所有产物放在 `tmp/2026-11-12-usage-stats/usage-p1-core/`，没有另建任务目录，无 GUI 截图。

原始备份只通过 `sqlite3 'file:<原始备份绝对路径>?immutable=1' '.backup <任务目录>/bench-index.sqlite'` 复制。其版本实际为 4；消息表/FTS 结构也较旧。为分离正常回填成本，prepare-bench.py 在当前 schema 中原样复制四张用量相关表；不把升级后空索引的打开时间冒充有数据时的核对时间。

| 场景 | 首次打开 | 完整账本 | 再打开 | 数据 |
|---|---:|---:|---:|---|
| 当前 schema，首次回填 | 356.324 ms | 17,485,824 B | 2.145 ms | 2,225 有用量 session / 129,372 调用 / 5,510 桶 |
| 版本 4 原样副本，抢救＋重建 | 1,347.400 ms | 17,637,376 B | 1.787 ms（索引为空） | 3,361 transcript 元数据 / 同样全部调用和桶 |

证据是 bench-final.log 和 bench-old-index.log；都用 release 的 ignored 测量测试运行。第二行包含无用量 transcript 的元数据，所以体积略大。相比 spec 预计 Rust 回填约 1 秒，正常回填为 36%；完整账本比单独调用表的 14 MB 大，包含其他表和页空隙。2.145 ms 核对符合小汇总表的 2–5 ms 估计。

## 交给协调者

- usage-p1-app 按上面的实际 API 接入；Today 上界取下一 UTC 整点，避免漏掉当前小时。spec 的调用示例已同步。
- AGENTS 的结构建议：core 增加 `ledger.rs`（独立、长期保留用量的账本）与 `usage.rs`（4:00 分日和内存聚合），app/CLI 同目录索引共享账本。
- AGENTS 的 core 规则建议：账本只增不删、独立迁移，不能套用索引的删库重建；Db 的扫描事务必须先提交账本再提交索引，账本错误不能被当成索引损坏；索引重建前先抢救旧用量。
- 不新增用户环境变量或 CLI 命令。ASM_LEDGER_BENCH_INDEX 只属于显式 ignored 测试，非产品接口。
- 已有 known-issues 中的“重建索引丢失已删 transcript 用量”在 core 行为上已覆盖；按 brief 由协调者在 P1 完整签收后删除并记录解决经过。
- 原始备份版本为 4，应保留到 usage-p1-import 签收。真实账本/索引尚未打开；本次结果不能替代真实数据核对和导入。
- 未改协调者的 AGENTS/known-issues/next-up/计划勾选/brief/status/tasks；未发现其他 agent 的未提交文件，无范围外修复。

## 踩过的坑

- 夏令时：带时区 DateTime 减四小时是减实际时长，不是墙上时间；必须先取 naive_local。
- rusqlite 的 SQLite INTEGER 按 i64 读写，不能直接 FromSql/ToSql u64；与现有索引用量代码相同地转换。
- 非事务状态单独重算小时汇总时，DELETE 成功而 INSERT 失败会留下空汇总；现已用自有事务保护，批次内复用外层事务。
- 损坏恢复先关闭连接，会暂时留下 None；若新建失败，不能把这个 None 当成“降级主动禁用”。ledger_required 保留打开时的意图，后续操作继续报错。
- 初次测试曾把两次仍在运行的命令输出写进同一个日志，产生空洞；最终验证使用独立 core-verified.log/app-verified.log，早期日志只作开发轨迹。
- parity 不仅受活跃会话写入影响，长查询期间滚动天数的截止线也会移动；即使固定文件快照也要核对两次开始时间。包装 Python 命令时必须只拦截目标脚本，不能误拦 Makefile 的 metadata 查询。
- 原始备份与 HEAD 的 schema 不同；仅改版本号不能让其成为兼容当前 schema 的回填基准，实测使用明确的四表投影并另测真实升级路径。

## 相关文档

- [总计划](../plans/2026-11-16-usage-stats/index.md) — 共用口径与分期依据。
- [账本 spec](../plans/2026-11-16-usage-stats/usage-ledger.md) — 本次实现依据；写回接口差异、Today 边界与实测值，不改勾选。
- [数据层设计](../docs/data-layer-design.md) — 增补账本现状及表结构/迁移指向。
- [M1 core 笔记](../notes/2026-09-17-m1-core-crate.md) — 模块规则与测试/比对依据。


## 协调者摘要（usage-p1-core-r1）

- 结果：R1、R2 和 `Ledger::usage(Some(ids))` 的 SQL 筛选均完成，公开 API 不变。
- 提交：代码、回归测试、spec 和本节同一提交；用 `git log -1 --format=%H -- crates/core/src/ledger.rs` 查询。
- R1：保留 SQLite 错误链，以 `LedgerFailure` 区分账本来源；BUSY/LOCKED 可被 `is_busy` 识别，账本损坏不触发索引删库。
- R2：三个事务外写入口先开内部事务，复用账本先提交、索引后提交；账本失败时索引与派生汇总回滚。
- SQL：单个 JSON 参数经 `json_each` 展开后按 id 连接，支持 40,000+ id；空列表直接返回，重复 id 与无元数据用量行为不变。
- 红灯证据：`r1-red-ledger.log`（4 失败）、`r1-red-cli.log`（1 失败）、`r1-red-classification.log`（1 失败），均在生产实现修改前取得。
- 绿灯证据：`cargo test -p agent-session-core`，372 通过 / 0 失败 / 5 忽略；`cargo test -p agent-session-manager`，162 通过 / 0 失败 / 2 忽略。
- 证据目录：`tmp/2026-11-12-usage-stats/usage-p1-core/`；全量输出为 `r1-core.log` 和 `r1-app.log`。
- 自己的决定：不用丢弃错误类型的字符串包装；不引入跨库事务或新的公共接口；schema 和账本版本不变。
- 未验证项：本轮未跑 GUI、真实数据导入或 parity；未改解析、输出格式、排序，未打开真实索引/账本或原始备份。
- 交给协调者：app 已有的 quiet 扫描 BUSY 分支可直接复用；无需 app 接口调整。范围外文件和协调者文件均未改。

### 返工回归覆盖

- SQLite BUSY/LOCKED 保留与 CORRUPT/NOTADB 来源隔离；共享账本锁导致写入和打开回填失败时，索引仍保留。
- CLI 在共享账本被锁时成功返回最后完成的索引并打印 busy 提示；解锁后下一次扫描读到新 session。
- standalone 用量写入失败后索引不留新行，重开后重试只记录一次；项目更新和 session 汇总写失败也回滚。
- SQL 选择覆盖超过 SQLite 参数上限的 id 集合、重复 id、缺失元数据、排除未选用量和空列表。

### 返工踩过的坑

- 将 `anyhow` 错误格式化后重新构造会丢失 SQLite 类型；添加文字应使用 `context`，来源标记单独控制损坏分类。
- autocommit 下在方法末尾 flush 已经太晚；事务必须在第一条索引写入之前开启。
