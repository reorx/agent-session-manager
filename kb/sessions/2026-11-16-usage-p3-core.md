---
created: 2026-11-16
tags: [usage, core, aggregation]
---

# Usage 第 3 期 core 聚合交接

## 协调者摘要（usage-p3-core）

- 完成范围、上一段、小时/日槽、过滤、分组、图表序列、移动平均和下拉条目。
- 提交：本文件与实现同一提交，hash 见最终交付消息。
- 测试先行：`red.log` 保留未实现接口时的编译失败；最终数量见下文。
- 真实账本副本 3,361 sessions / 5,510 桶，release 30 天 summary + series 平均 2.512 ms（1,000 次）。
- 证据与副本：`tmp/2026-11-12-usage-stats/usage-p3-core/`。
- 决定：保留公开 sessions，不加可能过期的费用缓存；旧 API 全部兼容。
- 不修改 app、不做 GUI；未运行 parity（未改解析、搜索、输出或排序口径）。
- 交给协调者：按接口派 P3 app，更新 AGENTS.md 和计划勾选。

## 接口

以下位于 `agent_session_core::usage`；时区参数均为 `&T: TimeZone`。

```rust
range_span(range: UsageRange, today: NaiveDate,
    custom: Option<(NaiveDate, NaiveDate)>, day: Option<NaiveDate>, tz: &T) -> Span
previous_span(range: UsageRange, span: Span, now_ms: i64, tz: &T) -> Option<Span>
hour_slots(day: NaiveDate, tz: &T) -> Vec<Span>
day_slots(span: Span, tz: &T) -> Vec<(NaiveDate, Span)>
moving_average(values: &[f64], window: usize) -> Vec<f64>
model_slot(model: &str) -> usize
UsageData::summary(&self, span: Span, filter: &UsageFilter) -> Summary
UsageData::summary_with(&self, span: Span, filter: &UsageFilter,
    price: impl Fn(&str) -> Option<ModelPrice>) -> Summary
UsageData::series(&self, slots: &[Span], filter: &UsageFilter) -> Vec<SlotTotals>
UsageData::series_with(&self, slots: &[Span], filter: &UsageFilter,
    price: impl Fn(&str) -> Option<ModelPrice>) -> Vec<SlotTotals>
UsageData::project_entries(&self) -> Vec<ProjectEntry>
UsageData::model_entries(&self) -> Vec<(String, u64)>
Totals::add(&mut self, other: &Totals)
SessionKind::from_metadata(is_background: bool, entrypoint: Option<&str>) -> Self
```

- `Span`、`Summary`、`ProjectUsage`、`SessionTotal`、`UsageFilter` 字段与 spec 一致；过滤默认三种类型全开。
- `SlotTotals { totals: Totals, by_model: [Totals; 5] }`，数组下标为 model_slot。
- `ProjectEntry { key: String, name: String, sessions: usize }`；条目按 key/model id 排序，全账本、不受当前过滤影响。
- `UsageRange` / `Metric` 可 serde，默认 Today / Cost；尚未接 app 保存。
- 模型归一化后过滤和分组；费用降序同价时保持 key/id 顺序。小时桶区间按 UTC 整小时向下取整，与旧 totals 一致，半小时时区的跨界桶归后一天。
- previous_span 仅无效区间返回 None；有效区间无数据时，调用方通过 summary.responses 判断。ThisMonth 上月短月截到月末；Day 总是比较前一天全天。moving_average 的 window=0 返回空。
- 与 spec 的差异仅费用没有预缓存，原因与实测见上文；价格注入接口是补充接口。

## 验证与用例对照

最终 `cargo test -p agent-session-core`：377 passed、0 failed、7 ignored；`cargo test -p agent-session-manager`：179 passed、0 failed、2 ignored。Usage 集成测试 13 passed、1 ignored（性能测试单独运行通过）。完整输出为 scratch 的 `core.log`、`app.log`、`bench.log`。

- `ranges_slots_and_previous_periods`：八种范围 4:00 边界、7/30 日、Custom 包含末日、24/23/25 小时槽、历史范围比较全天。
- `each_previous_range_has_exact_calendar_boundaries`：逐范围断言上一段精确起止时刻。
- `summary_filters_ranking_series_and_entries`：合计与分组守恒、项目/CLI/SDK/BG/响应模型过滤、跨日 session 范围内费用排名、账本标题保留、固定五色槽、条目、7 日均线。
- `injected_prices_and_half_hour_buckets_are_consistent`：注入价格、未知模型 token、三段 In、series/summary 一致、半小时时区跨界桶归属。
- 既有 usage 测试覆盖凌晨归日前日、缺失本地时间、价格表变化、未知价格计数、增量替换和逐调用计价一致性。
- `real_ledger_aggregation_timings` 是显式忽略测试，仅传副本绝对路径运行；加载不计入耗时。

## 交给协调者

更新 AGENTS.md 的 P3 core 实现状态；P3 app 可按上述签名接入。未发现范围外的工作区改动。未验证 GUI，按 brief 由独立任务负责。费用缓存是有实测支持的主动偏离，请审阅时核对。

## 踩过的坑

Cargo integration test 的当前目录在 crate 内，性能测试的账本路径必须用绝对路径；第一次相对路径报错，未触碰真实账本。

## 相关文档

- [Usage 视图计划](../plans/2026-11-16-usage-stats/usage-view.md) — 依据与实测回填。
- [数据层设计](../docs/data-layer-design.md) — 更新现状入口。
