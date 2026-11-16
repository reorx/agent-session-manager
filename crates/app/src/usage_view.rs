//! The Usage view: what the calls in a time range cost and how many tokens they took, by hour or
//! by day, by kind of token, model, project and session.
//!
//! Everything on screen comes from one `UsageSnapshot`, built by a pure function from the
//! in-memory usage data (`UsageData`, from the ledger), the range, the filters and the clock. It
//! is built again only when one of those changed (`AppRoot::refresh_usage`); render only reads its
//! strings. The chart (`usage_chart.rs`) draws `UsageSnapshot::chart`.

use std::time::Instant;

use agent_session_core::format::fmt_tokens;
use agent_session_core::model::Tokens;
use agent_session_core::pricing::display_name;
use agent_session_core::usage::{
    Metric, ProjectEntry, SlotTotals, Span, Totals, UsageData, UsageFilter, UsageRange, day_slots, day_start_ms, hour_slots, model_slot,
    moving_average, previous_span, range_span, usage_day,
};
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Weekday};
use gpui_kit::component::date_picker::DatePickerState;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::AppRoot;
use crate::model::{Filter, MainView};
use crate::resident::thousands;
use crate::settings::ViewState;
use crate::style::{self, FluentBuilder};
use crate::toolbar::{segment, segmented};

/// The key context of the view: `escape` leaves a day clicked on the chart there.
pub const KEY_CONTEXT: &str = "Usage";
/// The right end of the status bar in the Usage view: how calls are put on days.
pub const BASIS: &str = "by API response time · day starts 04:00";

/// The Usage view's part of `AppRoot`: what it shows and the snapshot built for it.
pub struct UsageState {
    pub range: UsageRange,
    /// The Custom range's first and last day, both included; `None` until one is picked.
    pub custom: Option<(NaiveDate, NaiveDate)>,
    /// The day clicked on the daily chart (`UsageRange::Day`); not saved.
    pub day: Option<NaiveDate>,
    /// The range that day was clicked in: Escape and the ✕ on its chip go back to it.
    pub back: UsageRange,
    pub metric: Metric,
    /// The view's keys (`escape`) and the app's (`⌃1`, `⌘F`, menus) start here while it is shown.
    pub focus: FocusHandle,
    /// Picks the Custom range; shown next to `Custom…` while Custom is on.
    pub date_picker: Entity<DatePickerState>,
    /// Built by `AppRoot::refresh_usage` while the view is shown; render only reads it.
    pub snapshot: UsageSnapshot,
    /// The chart bar under the mouse (`usage_chart.rs`).
    pub hover: Option<usize>,
    /// The Project dropdown: every project with calls in the ledger, transcripts gone or not.
    pub projects: Vec<ProjectEntry>,
    /// The Model dropdown: every model in the ledger and its calls, most calls first.
    pub models: Vec<(String, u64)>,
    /// Sessions with calls by kind (CLI, SDK, BG), for the Kind dropdown.
    pub kind_counts: [usize; 3],
}

impl UsageState {
    pub fn new(saved: &ViewState, date_picker: Entity<DatePickerState>, focus: FocusHandle) -> Self {
        // a clicked day is never saved; should one be, the view opens on today
        let range = if saved.usage_range == UsageRange::Day { UsageRange::Today } else { saved.usage_range };
        UsageState {
            range,
            custom: saved.usage_custom_range,
            day: None,
            back: range,
            metric: saved.usage_metric,
            focus,
            date_picker,
            snapshot: UsageSnapshot::default(),
            hover: None,
            projects: Vec::new(),
            models: Vec::new(),
            kind_counts: [0; 3],
        }
    }
}

/// Model colors of the chart, its legend and the model table, by `usage::model_slot`.
pub const MODEL_SLOT_NAMES: [&str; 5] = ["Opus 5.5", "Sonnet 5.5", "Fable 5.2", "Haiku 5.5", "Earlier models"];
/// Projects listed one by one in the project table; the rest share a row.
pub const TOP_PROJECTS: usize = 7;
/// Sessions in the most expensive sessions list.
pub const TOP_SESSIONS: usize = 6;
/// Days in the moving average of the daily chart: the day and the six before it.
pub const AVERAGE_DAYS: usize = 7;

/// What the view shows besides the toolbar and the status bar: the range, what the chart
/// measures, and the filters it shares with the session table.
#[derive(Clone, Debug)]
pub struct UsageQuery {
    pub range: UsageRange,
    /// The Custom range's first and last day; `None` until one is picked (this month so far).
    pub custom: Option<(NaiveDate, NaiveDate)>,
    /// The day clicked on the daily chart, for `UsageRange::Day`.
    pub day: Option<NaiveDate>,
    pub metric: Metric,
    pub filter: UsageFilter,
}

/// Why there is nothing to show, if there is not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Empty {
    #[default]
    No,
    /// The ledger holds no call yet (a new install, or the start-up scan has not ended).
    NoUsage,
    /// None of the ledger's calls is in the range and the filters.
    NoCalls,
    /// This run has no ledger: the whole view is one sentence.
    Unavailable,
}

impl Empty {
    /// What a table says when it has no rows.
    pub fn message(self) -> &'static str {
        match self {
            Empty::NoUsage => "No usage recorded yet",
            Empty::Unavailable => UNAVAILABLE,
            Empty::No | Empty::NoCalls => "No calls in this range",
        }
    }
}

pub const UNAVAILABLE: &str = "Usage ledger unavailable: it was written by a newer version of the app";

/// How a piece of a card's small text is colored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    /// More than the period before.
    Up,
    /// Less than the period before.
    Down,
    /// Calls left out of the cost.
    Warn,
}

/// One of the five cards: `COST`, `$203.69`, `yesterday by 16:00 $211.30 · ▼ 3.6%`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    pub label: SharedString,
    pub value: SharedString,
    pub sub: Vec<(SharedString, Tone)>,
}

impl Card {
    /// The small text as one string (tests, logs).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn sub_text(&self) -> String {
        self.sub.iter().map(|(text, _)| text.as_ref()).collect()
    }
}

/// A kind of token in the cost breakdown, which picks its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    CacheRead,
    CacheWrite,
    Output,
    Uncached,
    WebSearch,
}

/// A row of "Where the money goes".
#[derive(Clone, Debug, PartialEq)]
pub struct PartRow {
    pub part: Part,
    pub name: SharedString,
    pub tokens: SharedString,
    pub cost: SharedString,
    pub share: SharedString,
    /// Its share of the cost, for the stacked bar.
    pub fraction: f32,
}

/// A row of the model table; `slot` is `None` on the total row.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelRow {
    pub slot: Option<usize>,
    pub name: SharedString,
    pub calls: SharedString,
    pub input: SharedString,
    pub hit: SharedString,
    pub output: SharedString,
    pub cost: SharedString,
    pub share: SharedString,
}

/// A row of the project table; `key` is `None` on the `N other projects` row.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectRow {
    pub key: Option<SharedString>,
    pub name: SharedString,
    /// Its cost against the first row's, for the bar.
    pub fraction: f32,
    pub cost: SharedString,
    pub share: SharedString,
    pub sessions: SharedString,
    /// The Project filter is set to it.
    pub selected: bool,
}

/// A row of the most expensive sessions.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionLine {
    pub id: SharedString,
    pub short_id: SharedString,
    pub title: SharedString,
    pub project: SharedString,
    pub input: SharedString,
    /// What the session cost inside the range, not in all.
    pub cost: SharedString,
}

/// An entry of the chart's legend.
#[derive(Clone, Debug, PartialEq)]
pub enum Legend {
    /// A model color slot and its name.
    Model(usize, SharedString),
    /// The dashed box behind an hour's bar: the day before, same hour.
    Ghost,
    /// The daily chart's dashed line.
    Average,
    /// The hit rate's line.
    HitRate,
}

/// One bar of the chart: an hour of a single day, or a day.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartSlot {
    /// `04`, `05`, … by the hour; `11-16` (`Today` for today) by the day.
    pub label: SharedString,
    /// Hourly: the day shown. Daily: the slot's day, what a click on it drills into.
    pub day: NaiveDate,
    pub span: Span,
    /// A Saturday or Sunday (daily).
    pub weekend: bool,
    pub totals: Totals,
    /// By `usage::model_slot`: the stacked segments.
    pub by_model: [Totals; 5],
    /// Hourly: the same hour of the day before (the dashed box behind the bar).
    pub ghost: Option<Totals>,
    /// Daily, any metric but the hit rate: the mean of the day and the six before it.
    pub average: Option<f64>,
    /// Daily: today's bar, still filling (`so far`).
    pub partial: bool,
    /// Hourly, today: an hour still to come.
    pub future: bool,
}

impl ChartSlot {
    /// The bar's height in `metric` (a share of In for the hit rate; `None` without any In).
    pub fn value(&self, metric: Metric) -> Option<f64> {
        metric_value(&self.totals, metric)
    }

    /// The stacked segments in `metric`, bottom first (zero for a hit rate: it is a line).
    #[allow(dead_code)] // for usage_chart.rs
    pub fn stack(&self, metric: Metric) -> [f64; 5] {
        match metric {
            Metric::HitRate => [0.0; 5],
            _ => self.by_model.map(|t| metric_value(&t, metric).unwrap_or(0.0)),
        }
    }
}

/// What the chart draws, all in the units of `metric` (a fraction for the hit rate).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChartData {
    /// One day by the hour (24 bars, 23 or 25 on a DST day), else one bar a day.
    pub hourly: bool,
    pub metric: Metric,
    /// `By hour` / `By day`.
    pub title: SharedString,
    /// `Today, Mon Nov 16` / `Last 30 days · click a bar to see its hours`.
    pub subtitle: SharedString,
    pub slots: Vec<ChartSlot>,
    /// The value axis: `y_min` is 0 except for the hit rate.
    pub y_min: f64,
    pub y_max: f64,
    /// Grid lines and their labels, from `y_min` to `y_max`.
    pub ticks: Vec<(f64, SharedString)>,
    pub legend: Vec<Legend>,
    /// Today by the hour: where now is, in slots from the left edge (`11.88`), and `15:53`.
    pub now: Option<(f64, SharedString)>,
}

/// Everything the Usage view shows, formatted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageSnapshot {
    pub empty: Empty,
    /// The range's first and last moment (`[from, until)`).
    pub span: Option<Span>,
    /// `Today`, `Last 30 days`, `Nov 2 – Nov 8`, `Mon Nov 9`.
    pub range_label: SharedString,
    /// Cost, In, Out, Calls, Sessions.
    pub cards: Vec<Card>,
    pub chart: ChartData,
    pub parts: Vec<PartRow>,
    pub models: Vec<ModelRow>,
    pub projects: Vec<ProjectRow>,
    pub sessions: Vec<SessionLine>,
    /// The raw totals of the range (tests compare them with the cards).
    pub totals: Totals,
}

/// `UsageFilter` from the filters the session table shares: project, CLI / SDK / BG, model.
/// Hidden projects, sessions without prompts, favorites, History and the table's range do not
/// apply: every call cost money.
pub fn usage_filter(filter: &Filter) -> UsageFilter {
    UsageFilter {
        project: filter.project.as_ref().map(|p| p.to_string()),
        cli: filter.kinds.cli,
        sdk: filter.kinds.sdk,
        bg: filter.kinds.bg,
        model: filter.model.as_ref().map(|m| m.to_string()),
    }
}

/// The two filters pick the same calls (`UsageFilter` has no `PartialEq`).
pub fn same_filter(a: &UsageFilter, b: &UsageFilter) -> bool {
    (&a.project, a.cli, a.sdk, a.bg, &a.model) == (&b.project, b.cli, b.sdk, b.bg, &b.model)
}

/// `metric` of `t`: dollars, tokens, or the share of In read from the cache (`None` without In).
pub fn metric_value(t: &Totals, metric: Metric) -> Option<f64> {
    match metric {
        Metric::Cost => Some(t.cost.total()),
        Metric::In => Some(t.tokens.input_total() as f64),
        Metric::Out => Some(t.tokens.output as f64),
        Metric::HitRate => hit_rate(&t.tokens),
    }
}

fn hit_rate(t: &Tokens) -> Option<f64> {
    let input = t.input_total();
    (input > 0).then(|| t.cache_read as f64 / input as f64)
}

/// `$1,234.56`
pub fn fmt_money(usd: f64) -> String {
    let cents = (usd.max(0.0) * 100.0).round() as u64;
    format!("${}.{:02}", thousands(cents / 100), cents % 100)
}

/// `98.3%`
pub fn fmt_pct(fraction: f64) -> String {
    format!("{:.1}%", fraction * 100.0)
}

/// `▲ 3.6%` when `cur` is more than `prev`, `▼ 3.6%` when less; `None` when there is nothing
/// to compare with.
pub fn fmt_delta(cur: f64, prev: f64) -> Option<(String, Tone)> {
    if prev <= 0.0 {
        return None;
    }
    let change = (cur - prev) / prev;
    let tone = if change >= 0.0 { Tone::Up } else { Tone::Down };
    Some((format!("{} {:.1}%", if change >= 0.0 { "▲" } else { "▼" }, change.abs() * 100.0), tone))
}

/// A value of `metric` as the chart's tooltip and legend write it.
#[allow(dead_code)] // for usage_chart.rs
pub fn fmt_metric(metric: Metric, value: f64) -> String {
    match metric {
        Metric::Cost => fmt_money(value),
        Metric::In | Metric::Out => fmt_tokens(value.round() as u64),
        Metric::HitRate => fmt_pct(value),
    }
}

/// The label of a value axis line `step` apart from the next: `$300`, `$1.5k`, `$0.25`, `120M`,
/// `96%`.
pub fn fmt_axis(metric: Metric, value: f64, step: f64) -> String {
    match metric {
        Metric::Cost if value >= 1000.0 => {
            let k = value / 1000.0;
            if (k - k.round()).abs() < 1e-9 { format!("${k:.0}k") } else { format!("${k:.1}k") }
        }
        Metric::Cost if step < 1.0 => format!("${value:.2}"),
        Metric::Cost => format!("${value:.0}"),
        Metric::In | Metric::Out => fmt_tokens(value.round() as u64),
        Metric::HitRate if step < 0.01 => format!("{:.1}%", value * 100.0),
        Metric::HitRate => format!("{:.0}%", value * 100.0),
    }
}

/// `Today`, `Yesterday`, `7 days`, … : the toolbar's range buttons, in order.
pub const RANGE_BUTTONS: [(UsageRange, &str); 6] = [
    (UsageRange::Today, "Today"),
    (UsageRange::Yesterday, "Yesterday"),
    (UsageRange::Days7, "7 days"),
    (UsageRange::Days30, "30 days"),
    (UsageRange::ThisMonth, "This month"),
    (UsageRange::LastMonth, "Last month"),
];

/// `Cost`, `In`, `Out`, `Hit rate`: the chart's metric switch, in order.
pub const METRIC_BUTTONS: [(Metric, &str); 4] = [(Metric::Cost, "Cost"), (Metric::In, "In"), (Metric::Out, "Out"), (Metric::HitRate, "Hit rate")];

/// `Mon Nov 9`
pub fn fmt_day(day: NaiveDate) -> String {
    day.format("%a %b %-d").to_string()
}

impl UsageSnapshot {
    /// Everything the view shows for `q`, at `now_ms` on the usage day `today` in `tz`.
    /// `available`: `None` until the store said whether this run has a ledger.
    pub fn build<T: TimeZone>(data: &UsageData, available: Option<bool>, q: &UsageQuery, now_ms: i64, today: NaiveDate, tz: &T) -> Self {
        let span = range_span(q.range, today, q.custom, q.day, tz);
        let range_label = range_label(q, span, tz).into();
        let chart = chart(data, q, span, now_ms, today, tz);
        if available == Some(false) {
            return UsageSnapshot { empty: Empty::Unavailable, span: Some(span), range_label, chart, ..Default::default() };
        }
        let has_usage = available == Some(true) && data.sessions.values().any(|s| s.hours.iter().any(|h| h.responses > 0));
        if !has_usage {
            let cards = CARD_LABELS.iter().map(|label| Card { label: (*label).into(), value: "—".into(), sub: Vec::new() }).collect();
            return UsageSnapshot { empty: Empty::NoUsage, span: Some(span), range_label, cards, chart, ..Default::default() };
        }
        let summary = data.summary(span, &q.filter);
        let previous = previous_span(q.range, span, now_ms, tz).map(|p| (p, data.series(&[p], &q.filter).remove(0).totals));
        let comparison = previous.map(|(p, totals)| (comparison_label(q, span, p, tz), totals));
        let t = summary.totals;
        let total = t.cost.total();
        UsageSnapshot {
            empty: if t.responses == 0 { Empty::NoCalls } else { Empty::No },
            span: Some(span),
            range_label,
            cards: cards(&summary, comparison.as_ref()),
            chart,
            parts: parts(&t),
            models: models(&summary.by_model, &t),
            projects: projects(&summary.by_project, total, q.filter.project.as_deref()),
            sessions: summary.by_session.iter().take(TOP_SESSIONS).map(session_line).collect(),
            totals: t,
        }
    }
}

const CARD_LABELS: [&str; 5] = ["Cost", "In", "Out", "Calls", "Sessions"];

/// The cards for `summary`; `comparison`: what the period before is called and its totals.
fn cards(summary: &agent_session_core::usage::Summary, comparison: Option<&(String, Totals)>) -> Vec<Card> {
    let t = &summary.totals;
    let earlier = comparison.filter(|(_, before)| before.responses > 0);
    let compare = |value: f64, before: &dyn Fn(&Totals) -> f64, fmt: &dyn Fn(f64) -> String| -> Vec<(SharedString, Tone)> {
        let Some((label, totals)) = earlier else { return vec![("no earlier data".into(), Tone::Plain)] };
        let prev = before(totals);
        let mut sub = vec![(format!("{label} {}", fmt(prev)).into(), Tone::Plain)];
        if let Some((delta, tone)) = fmt_delta(value, prev) {
            sub.push((" · ".into(), Tone::Plain));
            sub.push((delta.into(), tone));
        }
        sub
    };
    let cost = t.cost.total();
    let mut cost_sub = compare(cost, &|b| b.cost.total(), &fmt_money);
    if t.unpriced_responses > 0 {
        cost_sub.push((format!(" · {} calls unpriced", thousands(t.unpriced_responses)).into(), Tone::Warn));
    }
    let tokens = &t.tokens;
    let input_sub = match hit_rate(tokens) {
        Some(hit) => format!(
            "cache read {} · cache write {} · uncached {}",
            fmt_pct(hit),
            fmt_tokens(tokens.cache_write_5m + tokens.cache_write_1h),
            fmt_tokens(tokens.input)
        ),
        None => String::new(),
    };
    let output = tokens.output as f64;
    let per_call = if t.responses > 0 { format!("{} per call", fmt_money(cost / t.responses as f64)) } else { String::new() };
    let projects = match summary.projects {
        1 => "1 project".to_string(),
        n => format!("{n} projects"),
    };
    let plain = |text: String| if text.is_empty() { Vec::new() } else { vec![(text.into(), Tone::Plain)] };
    vec![
        Card { label: "Cost".into(), value: fmt_money(cost).into(), sub: cost_sub },
        Card { label: "In".into(), value: fmt_tokens(tokens.input_total()).into(), sub: plain(input_sub) },
        Card { label: "Out".into(), value: fmt_tokens(tokens.output).into(), sub: compare(output, &|b| b.tokens.output as f64, &|v| fmt_tokens(v as u64)) },
        Card { label: "Calls".into(), value: thousands(t.responses).into(), sub: plain(per_call) },
        Card { label: "Sessions".into(), value: thousands(summary.sessions as u64).into(), sub: plain(projects) },
    ]
}

/// "Where the money goes": cache read, cache write (both lifetimes), output, uncached input, and
/// web searches when there were any.
fn parts(t: &Totals) -> Vec<PartRow> {
    let (c, k) = (&t.cost, &t.tokens);
    let total = c.total();
    let mut rows = vec![
        (Part::CacheRead, "Cache read", fmt_tokens(k.cache_read), c.cache_read),
        (Part::CacheWrite, "Cache write", fmt_tokens(k.cache_write_5m + k.cache_write_1h), c.cache_write_5m + c.cache_write_1h),
        (Part::Output, "Output", fmt_tokens(k.output), c.output),
        (Part::Uncached, "Uncached input", fmt_tokens(k.input), c.input),
    ];
    if c.web_search > 0.0 || k.web_search > 0 {
        rows.push((Part::WebSearch, "Web search", format!("{} searches", thousands(k.web_search)), c.web_search));
    }
    rows.into_iter()
        .map(|(part, name, tokens, cost)| PartRow {
            part,
            name: name.into(),
            tokens: tokens.into(),
            cost: fmt_money(cost).into(),
            share: share(cost, total).into(),
            fraction: if total > 0.0 { (cost / total) as f32 } else { 0.0 },
        })
        .collect()
}

fn share(part: f64, total: f64) -> String {
    if total > 0.0 { fmt_pct(part / total) } else { "—".into() }
}

/// One row per model by cost (fast and long prompts are in their model's row), then the total.
fn models(by_model: &[(String, Totals)], total: &Totals) -> Vec<ModelRow> {
    if by_model.is_empty() {
        return Vec::new();
    }
    let cost = total.cost.total();
    let row = |slot: Option<usize>, name: String, t: &Totals| ModelRow {
        slot,
        name: name.into(),
        calls: thousands(t.responses).into(),
        input: fmt_tokens(t.tokens.input_total()).into(),
        hit: hit_rate(&t.tokens).map_or("—".into(), fmt_pct).into(),
        output: fmt_tokens(t.tokens.output).into(),
        // a model without a price: its calls cost nothing we know of
        cost: if t.responses > 0 && t.unpriced_responses == t.responses { "—".into() } else { fmt_money(t.cost.total()).into() },
        share: if slot.is_some() { share(t.cost.total(), cost).into() } else { "".into() },
    };
    let mut rows: Vec<ModelRow> = by_model.iter().map(|(model, t)| row(Some(model_slot(model)), display_name(model), t)).collect();
    rows.push(row(None, "Total".into(), total));
    rows
}

/// The `TOP_PROJECTS` most expensive projects, then `N other projects` for the rest.
fn projects(by_project: &[agent_session_core::usage::ProjectUsage], total: f64, picked: Option<&str>) -> Vec<ProjectRow> {
    let top = by_project.first().map_or(0.0, |p| p.totals.cost.total());
    let mut rows: Vec<ProjectRow> = by_project
        .iter()
        .take(TOP_PROJECTS)
        .map(|p| {
            let cost = p.totals.cost.total();
            ProjectRow {
                key: Some(p.key.clone().into()),
                name: p.name.clone().into(),
                fraction: if top > 0.0 { (cost / top) as f32 } else { 0.0 },
                cost: fmt_money(cost).into(),
                share: share(cost, total).into(),
                sessions: thousands(p.sessions as u64).into(),
                selected: picked == Some(p.key.as_str()),
            }
        })
        .collect();
    let rest = &by_project[by_project.len().min(TOP_PROJECTS)..];
    if !rest.is_empty() {
        let cost: f64 = rest.iter().map(|p| p.totals.cost.total()).sum();
        let sessions: usize = rest.iter().map(|p| p.sessions).sum();
        rows.push(ProjectRow {
            key: None,
            name: format!("{} other projects", rest.len()).into(),
            fraction: 0.0,
            cost: fmt_money(cost).into(),
            share: share(cost, total).into(),
            sessions: thousands(sessions as u64).into(),
            selected: false,
        });
    }
    rows
}

fn session_line(s: &agent_session_core::usage::SessionTotal) -> SessionLine {
    SessionLine {
        id: s.id.clone().into(),
        short_id: s.id.chars().take(8).collect::<String>().into(),
        title: if s.title.trim().is_empty() { "(untitled)".into() } else { s.title.clone().into() },
        project: s.project_name.clone().into(),
        input: fmt_tokens(s.totals.tokens.input_total()).into(),
        cost: fmt_money(s.totals.cost.total()).into(),
    }
}

fn local<T: TimeZone>(ms: i64, tz: &T) -> NaiveDateTime {
    tz.timestamp_millis_opt(ms).single().expect("valid timestamp").naive_local()
}

/// The span's last usage day (its end is the start of the day after).
fn last_day<T: TimeZone>(span: Span, tz: &T) -> NaiveDate {
    usage_day(span.until_ms - 1, tz)
}

fn range_label<T: TimeZone>(q: &UsageQuery, span: Span, tz: &T) -> String {
    let (first, last) = (usage_day(span.from_ms, tz), last_day(span, tz));
    match q.range {
        UsageRange::Today => "Today".into(),
        UsageRange::Yesterday => "Yesterday".into(),
        UsageRange::Days7 => "Last 7 days".into(),
        UsageRange::Days30 => "Last 30 days".into(),
        UsageRange::ThisMonth => "This month".into(),
        UsageRange::LastMonth => "Last month".into(),
        UsageRange::Custom if first == last => fmt_day(first),
        UsageRange::Custom => format!("{} – {}", first.format("%b %-d"), last.format("%b %-d")),
        UsageRange::Day => fmt_day(first),
    }
}

/// What the cards call the period they compare with: `yesterday by 16:00`, `prior 7 days`, …
fn comparison_label<T: TimeZone>(q: &UsageQuery, span: Span, previous: Span, tz: &T) -> String {
    let days = (last_day(span, tz) - usage_day(span.from_ms, tz)).num_days() + 1;
    match q.range {
        UsageRange::Today => format!("yesterday by {}", local(previous.until_ms, tz).format("%H:%M")),
        UsageRange::Yesterday | UsageRange::Day => "day before".into(),
        UsageRange::Days7 | UsageRange::Days30 | UsageRange::Custom if days == 1 => "day before".into(),
        UsageRange::Days7 | UsageRange::Days30 | UsageRange::Custom => format!("prior {days} days"),
        UsageRange::ThisMonth => "same days last month".into(),
        UsageRange::LastMonth => "month before".into(),
    }
}

/// The chart: by the hour for a range of one day, else by the day.
fn chart<T: TimeZone>(data: &UsageData, q: &UsageQuery, span: Span, now_ms: i64, today: NaiveDate, tz: &T) -> ChartData {
    let first = usage_day(span.from_ms, tz);
    let hourly = first == last_day(span, tz);
    let metric = q.metric;
    let mut slots = if hourly { hour_chart(data, q, first, now_ms, today, tz) } else { day_chart(data, q, span, today, tz) };
    if metric == Metric::HitRate {
        for slot in &mut slots {
            slot.average = None;
        }
    }
    let (y_min, y_max, step) = value_axis(&slots, metric);
    let count = ((y_max - y_min) / step).round() as usize;
    let ticks = (0..=count).map(|i| if i == count { y_max } else { y_min + step * i as f64 }).map(|v| (v, fmt_axis(metric, v, step).into())).collect();
    let mut legend: Vec<Legend> = Vec::new();
    if metric == Metric::HitRate {
        legend.push(Legend::HitRate);
    } else {
        for slot in 0..MODEL_SLOT_NAMES.len() {
            if slots.iter().any(|s| s.by_model[slot].responses > 0) {
                legend.push(Legend::Model(slot, MODEL_SLOT_NAMES[slot].into()));
            }
        }
        legend.push(if hourly { Legend::Ghost } else { Legend::Average });
    }
    let now = (hourly && first == today)
        .then(|| slots.iter().position(|s| s.span.from_ms <= now_ms && now_ms < s.span.until_ms))
        .flatten()
        .map(|ix| {
            let s = slots[ix].span;
            (ix as f64 + (now_ms - s.from_ms) as f64 / (s.until_ms - s.from_ms) as f64, local(now_ms, tz).format("%H:%M").to_string().into())
        });
    let label = range_label(q, span, tz);
    let subtitle = if !hourly {
        format!("{label} · click a bar to see its hours")
    } else if matches!(q.range, UsageRange::Today | UsageRange::Yesterday) {
        format!("{label}, {}", fmt_day(first))
    } else {
        label
    };
    ChartData {
        hourly,
        metric,
        title: if hourly { "By hour" } else { "By day" }.into(),
        subtitle: subtitle.into(),
        slots,
        y_min,
        y_max,
        ticks,
        legend,
        now,
    }
}

fn slot_of(span: Span, label: String, day: NaiveDate, t: SlotTotals) -> ChartSlot {
    ChartSlot {
        label: label.into(),
        day,
        span,
        weekend: false,
        totals: t.totals,
        by_model: t.by_model,
        ghost: None,
        average: None,
        partial: false,
        future: false,
    }
}

/// The hours of `day`, each with the same hour of the day before behind it.
fn hour_chart<T: TimeZone>(data: &UsageData, q: &UsageQuery, day: NaiveDate, now_ms: i64, today: NaiveDate, tz: &T) -> Vec<ChartSlot> {
    let spans = hour_slots(day, tz);
    let before = data.series(&hour_slots(day - Duration::days(1), tz), &q.filter);
    let series = data.series(&spans, &q.filter);
    spans
        .iter()
        .zip(series)
        .enumerate()
        .map(|(ix, (span, t))| {
            let mut slot = slot_of(*span, local(span.from_ms, tz).format("%H").to_string(), day, t);
            slot.ghost = before.get(ix).map(|b| b.totals);
            slot.future = day == today && span.from_ms > now_ms;
            slot
        })
        .collect()
}

/// A bar a day, with the moving average of the day and the six before (outside the range too).
fn day_chart<T: TimeZone>(data: &UsageData, q: &UsageQuery, span: Span, today: NaiveDate, tz: &T) -> Vec<ChartSlot> {
    let lead = (AVERAGE_DAYS - 1) as i64;
    let first = usage_day(span.from_ms, tz);
    let wider = Span { from_ms: day_start_ms(first - Duration::days(lead), tz), until_ms: span.until_ms };
    let days = day_slots(wider, tz);
    let series = data.series(&days.iter().map(|(_, s)| *s).collect::<Vec<_>>(), &q.filter);
    let values: Vec<f64> = series.iter().map(|t| metric_value(&t.totals, q.metric).unwrap_or(0.0)).collect();
    let average = moving_average(&values, AVERAGE_DAYS);
    days.into_iter()
        .zip(series)
        .zip(average)
        .skip(lead as usize)
        .map(|(((day, span), t), average)| {
            let label = if day == today { "Today".to_string() } else { day.format("%m-%d").to_string() };
            let mut slot = slot_of(span, label, day, t);
            slot.weekend = matches!(day.weekday(), Weekday::Sat | Weekday::Sun);
            slot.partial = day == today;
            slot.average = Some(average);
            slot
        })
        .collect()
}

/// `(min, max, step)` of the value axis: from 0 to a round number above every bar, average and
/// box behind a bar; for the hit rate from a little under the lowest point to 100%.
fn value_axis(slots: &[ChartSlot], metric: Metric) -> (f64, f64, f64) {
    if metric == Metric::HitRate {
        let lowest = slots.iter().filter_map(|s| s.value(metric)).fold(f64::INFINITY, f64::min);
        let lowest = if lowest.is_finite() { lowest } else { 0.9 };
        let floor = ((lowest * 100.0 - 1.0).floor() / 100.0).max(0.0);
        let step = nice_step(1.0 - floor, 4);
        return ((floor / step).floor() * step, 1.0, step);
    }
    let highest = slots
        .iter()
        .flat_map(|s| [s.value(metric), s.average, s.ghost.and_then(|g| metric_value(&g, metric))])
        .flatten()
        .fold(0.0, f64::max);
    let top = if highest > 0.0 { highest } else { 1.0 };
    let step = nice_step(top, 4);
    (0.0, (top / step).ceil() * step, step)
}

/// A round step (1, 2, 2.5, 5 × 10ⁿ) that splits `span` into about `parts`.
fn nice_step(span: f64, parts: u32) -> f64 {
    if span <= 0.0 {
        return 1.0;
    }
    let raw = span / parts as f64;
    let power = 10f64.powf(raw.log10().floor());
    let f = raw / power;
    let nice = if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 2.5 {
        2.5
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * power
}

impl AppRoot {
    /// What the view shows right now.
    pub(crate) fn usage_query(&self, cx: &App) -> UsageQuery {
        let u = &self.usage_view;
        UsageQuery { range: u.range, custom: u.custom, day: u.day, metric: u.metric, filter: usage_filter(&self.current_filter(cx)) }
    }

    /// Build the view again: after new usage, a filter, range or metric change, a new usage day,
    /// the timer (the clock moved), or the view coming on screen (`set_view` builds it then, so
    /// nothing is built while the session table is shown). Nothing on screen changes when nothing
    /// did.
    pub(crate) fn refresh_usage(&mut self, cx: &mut Context<Self>) {
        if self.view != MainView::Usage {
            return;
        }
        let started = Instant::now();
        let query = self.usage_query(cx);
        let snapshot = UsageSnapshot::build(&self.usage, self.usage_available, &query, crate::now_ms(), self.usage_day, &Local);
        let u = &mut self.usage_view;
        u.projects = self.usage.project_entries();
        u.projects.sort_by_cached_key(|p| p.name.to_lowercase());
        u.models = self.usage.model_entries();
        u.models.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        u.kind_counts = usage_kind_counts(&self.usage);
        if snapshot != u.snapshot {
            let t = &snapshot.totals;
            println!(
                "usage view: {} · {} calls · {} · built in {:.1}ms",
                snapshot.range_label,
                t.responses,
                fmt_money(t.cost.total()),
                started.elapsed().as_secs_f64() * 1000.0
            );
            u.snapshot = snapshot;
            cx.notify();
        }
    }

    /// A range button, `Custom…`, or the status bar's Today. Leaves a clicked day.
    pub fn set_usage_range(&mut self, range: UsageRange, cx: &mut Context<Self>) {
        let u = &mut self.usage_view;
        u.range = range;
        u.day = None;
        u.back = range;
        self.refresh_usage(cx);
        self.persist(cx);
        cx.notify();
    }

    /// A range picked in the Usage view's date picker: both days included.
    pub fn set_usage_custom_range(&mut self, start: NaiveDate, end: NaiveDate, cx: &mut Context<Self>) {
        self.usage_view.custom = Some((start.min(end), start.max(end)));
        self.set_usage_range(UsageRange::Custom, cx);
    }

    /// What the chart shows: Cost, In, Out or Hit rate.
    pub fn set_usage_metric(&mut self, metric: Metric, cx: &mut Context<Self>) {
        self.usage_view.metric = metric;
        self.refresh_usage(cx);
        self.persist(cx);
        cx.notify();
    }

    /// A click on a day's bar of the daily chart: that day by the hour, until Escape or the ✕ on
    /// its chip. A day without calls does nothing (false).
    pub fn drill_into_day(&mut self, day: NaiveDate, cx: &mut Context<Self>) -> bool {
        let chart = &self.usage_view.snapshot.chart;
        if chart.hourly || !chart.slots.iter().any(|s| s.day == day && s.totals.responses > 0) {
            return false;
        }
        let u = &mut self.usage_view;
        if u.range != UsageRange::Day {
            u.back = u.range;
        }
        u.range = UsageRange::Day;
        u.day = Some(day);
        u.hover = None;
        self.refresh_usage(cx);
        cx.notify();
        true
    }

    /// Escape, or the ✕ on a clicked day's chip: back to the range it was clicked in.
    pub fn leave_day(&mut self, cx: &mut Context<Self>) {
        if self.usage_view.range != UsageRange::Day {
            return;
        }
        let u = &mut self.usage_view;
        u.range = u.back;
        u.day = None;
        u.hover = None;
        self.refresh_usage(cx);
        cx.notify();
    }
}

/// The id of the chart's area: `usage_chart.rs` draws into it.
pub const CHART_ID: &str = "usage-chart";
/// The chart area's height.
pub const CHART_H: f32 = 236.;
/// Below this window width the side-by-side panels stack.
const NARROW_W: f32 = 960.;
/// The width of a number column of the tables.
const NUM_W: f32 = 76.;

impl AppRoot {
    /// The Usage view under the toolbar: five cards, the chart, then "Where the money goes" next
    /// to "By model" and "By project" next to "Most expensive sessions"; stacked when narrow.
    pub(crate) fn render_usage(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let u = &self.usage_view;
        let snap = &u.snapshot;
        let base = div()
            .id("usage-view")
            .key_context(KEY_CONTEXT)
            .track_focus(&u.focus)
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(style::white())
            .text_color(style::text());
        if snap.empty == Empty::Unavailable {
            return base.child(h_flex().size_full().justify_center().items_center().text_size(px(13.)).text_color(style::muted()).child(UNAVAILABLE));
        }
        let narrow = window.viewport_size().width < px(NARROW_W);
        base.child(
            div().id("usage-scroll").size_full().overflow_y_scrollbar().child(
                v_flex()
                    .w_full()
                    .child(render_cards(&snap.cards, cx))
                    .child(self.render_chart_panel(cx))
                    .child(grid2(narrow, self.render_parts(cx), self.render_models(cx)))
                    .child(grid2(narrow, self.render_projects(cx), self.render_sessions(cx))),
            ),
        )
    }

    /// The chart's panel: title, range, metric switch and legend over the chart area, which
    /// `usage_chart.rs` fills.
    fn render_chart_panel(&self, cx: &mut Context<Self>) -> Div {
        let u = &self.usage_view;
        let chart = &u.snapshot.chart;
        let mut metrics = segmented();
        for (ix, (metric, label)) in METRIC_BUTTONS.iter().copied().enumerate() {
            metrics = metrics.child(segment(("usage-metric", ix), label, u.metric == metric).on_click(cx.listener(move |this, _, _, cx| this.set_usage_metric(metric, cx))));
        }
        let no_usage = u.snapshot.empty == Empty::NoUsage;
        let area = div().id(CHART_ID).w_full().h(px(CHART_H)).flex_none().when(no_usage, |el| {
            el.flex().items_center().justify_center().text_size(px(12.5)).text_color(style::muted()).child(Empty::NoUsage.message())
        });
        panel()
            .border_b_1()
            .border_color(style::hairline())
            .child(
                h_flex()
                    .w_full()
                    .gap(px(10.))
                    .items_center()
                    .mb(px(10.))
                    .child(panel_title(chart.title.clone()))
                    .child(div().text_size(px(12.)).text_color(style::muted()).child(chart.subtitle.clone()))
                    .child(metrics)
                    .child(div().flex_1())
                    .child(render_legend(&chart.legend)),
            )
            .child(area)
    }

    /// "Where the money goes": a stacked bar of the cost by kind of token, then its table.
    fn render_parts(&self, cx: &App) -> Div {
        let snap = &self.usage_view.snapshot;
        let mut bar = h_flex().w_full().h(px(14.)).gap(px(2.)).mb(px(10.));
        for row in snap.parts.iter().filter(|r| r.fraction > 0.0) {
            bar = bar.child(div().h_full().rounded(px(2.)).bg(part_color(row.part)).flex_grow(row.fraction).flex_basis(px(0.)));
        }
        let head = header_row("Kind", &["Tokens", "Cost", "Share"]);
        // without calls the rows would be all zeros: the note says it
        let rows = snap.parts.iter().filter(|_| snap.totals.responses > 0).map(|r| {
            table_row(false, false)
                .child(name_cell(h_flex().gap(px(7.)).child(swatch(part_color(r.part))).child(r.name.clone())))
                .child(num_cell(r.tokens.clone(), cx))
                .child(num_cell(r.cost.clone(), cx))
                .child(num_cell(r.share.clone(), cx))
        });
        panel()
            .child(panel_head("Where the money goes", ""))
            .when(!snap.parts.is_empty() && snap.totals.cost.total() > 0.0, |el| el.child(bar))
            .child(head)
            .children(rows)
            .when(snap.parts.is_empty() || snap.totals.responses == 0, |el| el.child(empty_note(snap.empty)))
    }

    /// One row a model by cost, then the total.
    fn render_models(&self, cx: &App) -> Div {
        let snap = &self.usage_view.snapshot;
        let rows = snap.models.iter().map(|r| {
            let total = r.slot.is_none();
            let name = match r.slot {
                Some(slot) => h_flex().gap(px(7.)).min_w_0().child(swatch(style::model_color(slot))).child(div().truncate().child(r.name.clone())),
                None => h_flex().child(r.name.clone()),
            };
            table_row(total, false)
                .child(name_cell(name))
                .child(num_cell(r.calls.clone(), cx))
                .child(num_cell(r.input.clone(), cx))
                .child(num_cell(r.hit.clone(), cx))
                .child(num_cell(r.output.clone(), cx))
                .child(num_cell(r.cost.clone(), cx))
                .child(num_cell(r.share.clone(), cx))
        });
        panel()
            .child(panel_head("By model", ""))
            .child(header_row("Model", &["Calls", "In", "Hit", "Out", "Cost", "Share"]))
            .children(rows)
            .when(snap.models.is_empty(), |el| el.child(empty_note(snap.empty)))
    }

    /// The most expensive projects; a click on one sets the Project filter to it (again: unsets).
    fn render_projects(&self, cx: &mut Context<Self>) -> Div {
        let snap = &self.usage_view.snapshot;
        let rows: Vec<AnyElement> = snap
            .projects
            .iter()
            .enumerate()
            .map(|(ix, r)| {
                let track = div().w(px(90.)).h(px(6.)).flex_none().rounded(px(3.)).bg(style::row_border()).when(r.key.is_some(), |el| {
                    el.child(div().h_full().rounded(px(3.)).bg(style::share_bar()).w(relative(r.fraction.clamp(0.0, 1.0))))
                });
                let row = table_row(false, r.selected)
                    .child(name_cell(div().truncate().when(r.key.is_none(), |el| el.text_color(style::muted())).child(r.name.clone())))
                    .child(div().w(px(100.)).flex_none().flex().justify_end().child(track))
                    .child(num_cell(r.cost.clone(), cx))
                    .child(num_cell(r.share.clone(), cx))
                    .child(num_cell(r.sessions.clone(), cx));
                match r.key.clone() {
                    Some(key) => {
                        let picked = r.selected;
                        div()
                            .id(("usage-project-row", ix))
                            .cursor_pointer()
                            .hover(|s| s.bg(style::hover_bg()))
                            .child(row)
                            .on_click(cx.listener(move |this, _, _, cx| this.set_project((!picked).then(|| key.clone()), cx)))
                            .into_any_element()
                    }
                    None => row.into_any_element(),
                }
            })
            .collect();
        panel()
            .border_t_1()
            .border_color(style::hairline())
            .child(panel_head("By project", "click a project to filter by it"))
            .child(header_row("Project", &["", "Cost", "Share", "Sessions"]))
            .children(rows)
            .when(snap.projects.is_empty(), |el| el.child(empty_note(snap.empty)))
    }

    /// The sessions that cost the most inside the range.
    fn render_sessions(&self, cx: &App) -> Div {
        let snap = &self.usage_view.snapshot;
        let rows = snap.sessions.iter().map(|s| {
            let cell = h_flex()
                .gap(px(8.))
                .min_w_0()
                .child(div().flex_none().font_family(style::mono(cx)).text_size(px(11.5)).text_color(style::accent()).child(s.short_id.clone()))
                .child(div().min_w_0().truncate().child(s.title.clone()))
                .child(div().flex_none().max_w(px(140.)).truncate().text_size(px(11.5)).text_color(style::muted()).child(s.project.clone()));
            table_row(false, false).child(name_cell(cell)).child(num_cell(s.input.clone(), cx)).child(num_cell(s.cost.clone(), cx))
        });
        panel()
            .border_t_1()
            .border_color(style::hairline())
            .child(panel_head("Most expensive sessions", "cost inside the range"))
            .child(header_row("Session", &["In", "Cost"]))
            .children(rows)
            .when(snap.sessions.is_empty(), |el| el.child(empty_note(snap.empty)))
    }
}

/// The five cards in a row.
fn render_cards(cards: &[Card], cx: &App) -> Div {
    let mut row = h_flex().w_full().items_stretch().border_b_1().border_color(style::hairline());
    for (ix, card) in cards.iter().enumerate() {
        // one text that wraps inside the card, its pieces colored by their tone
        let mut text = String::new();
        let mut marks = Vec::new();
        for (piece, tone) in &card.sub {
            let color = match tone {
                Tone::Plain => None,
                Tone::Up => Some(style::cost_up()),
                Tone::Down => Some(style::cost_down()),
                Tone::Warn => Some(style::warn()),
            };
            if let Some(color) = color {
                marks.push((text.len()..text.len() + piece.len(), HighlightStyle { color: Some(color), ..Default::default() }));
            }
            text.push_str(piece);
        }
        let sub = StyledText::new(text).with_highlights(marks);
        row = row.child(
            v_flex()
                .flex_1()
                .min_w_0()
                .px(px(20.))
                .pt(px(16.))
                .pb(px(15.))
                .gap(px(4.))
                .when(ix > 0, |el| el.border_l_1().border_color(style::hairline()))
                .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(style::label()).child(card.label.to_uppercase()))
                .child(div().font_family(style::mono(cx)).text_size(px(24.)).font_weight(FontWeight::SEMIBOLD).line_height(px(30.)).child(card.value.clone()))
                .child(div().w_full().text_size(px(12.)).min_h(px(16.)).text_color(style::text_secondary()).child(sub)),
        );
    }
    row
}

fn render_legend(legend: &[Legend]) -> Div {
    let mut row = h_flex().gap(px(12.)).text_size(px(11.5)).text_color(style::text_secondary());
    for entry in legend {
        let (mark, name): (Div, SharedString) = match entry {
            Legend::Model(slot, name) => (swatch(style::model_color(*slot)), name.clone()),
            Legend::Ghost => (div().size(px(9.)).rounded(px(2.)).border_1().border_dashed().border_color(style::chart_ghost()), "day before".into()),
            Legend::Average => (div().w(px(14.)).h(px(0.)).border_t_2().border_dashed().border_color(style::chart_ghost()), "7-day average".into()),
            Legend::HitRate => (div().w(px(14.)).h(px(2.)).bg(style::model_color(0)), "hit rate".into()),
        };
        row = row.child(h_flex().gap(px(5.)).items_center().child(mark).child(name));
    }
    row
}

/// Side by side with a line between them, or one above the other when the window is narrow.
fn grid2(narrow: bool, left: Div, right: Div) -> Div {
    if narrow {
        v_flex().w_full().child(left).child(right)
    } else {
        h_flex().w_full().items_start().child(left.flex_1().min_w_0()).child(right.flex_1().min_w_0().border_l_1().border_color(style::hairline()))
    }
}

fn panel() -> Div {
    v_flex().w_full().px(px(20.)).pt(px(16.)).pb(px(18.))
}

fn panel_title(title: impl Into<SharedString>) -> Div {
    div().flex_none().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).child(title.into())
}

fn panel_head(title: &'static str, sub: &'static str) -> Div {
    h_flex()
        .gap(px(10.))
        .items_baseline()
        .mb(px(10.))
        .child(panel_title(title))
        .when(!sub.is_empty(), |el| el.child(div().text_size(px(12.)).text_color(style::muted()).child(sub)))
}

/// The header: the first column on the left, the number columns right-aligned.
fn header_row(first: &'static str, numbers: &[&'static str]) -> Div {
    let mut row = h_flex()
        .w_full()
        .h(px(24.))
        .items_center()
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(style::head_fg())
        .border_b_1()
        .border_color(style::row_border())
        .child(div().flex_1().min_w_0().child(first));
    for name in numbers {
        let w = if name.is_empty() { 100. } else { NUM_W };
        row = row.child(div().w(px(w)).flex_none().text_right().child(*name));
    }
    row
}

/// A row: `total` is bold with a line above, `selected` is lit.
fn table_row(total: bool, selected: bool) -> Div {
    h_flex()
        .w_full()
        .h(px(26.))
        .items_center()
        .text_size(px(12.5))
        .border_b_1()
        .border_color(style::row_border())
        .when(total, |el| el.font_weight(FontWeight::SEMIBOLD).border_t_1().border_color(style::border()))
        .when(selected, |el| el.bg(style::lit_bg()))
}

fn name_cell(content: impl IntoElement) -> Div {
    div().flex_1().min_w_0().pr(px(8.)).child(content)
}

fn num_cell(text: SharedString, cx: &App) -> Div {
    div().w(px(NUM_W)).flex_none().text_right().font_family(style::mono(cx)).text_size(px(12.)).child(text)
}

fn swatch(color: Hsla) -> Div {
    div().flex_none().size(px(9.)).rounded(px(2.)).bg(color)
}

/// Under a table without rows.
fn empty_note(empty: Empty) -> Div {
    div().py(px(18.)).text_size(px(12.5)).text_color(style::muted()).child(empty.message())
}

fn part_color(part: Part) -> Hsla {
    match part {
        Part::CacheRead => style::token_cache_read(),
        Part::CacheWrite => style::token_cache_write(),
        Part::Output => style::token_output(),
        Part::Uncached => style::token_input(),
        Part::WebSearch => style::token_web_search(),
    }
}

/// Sessions with calls in the ledger by kind: CLI, SDK, BG.
fn usage_kind_counts(data: &UsageData) -> [usize; 3] {
    use agent_session_core::session::SessionKind;
    let mut counts = [0; 3];
    for s in data.sessions.values().filter(|s| s.hours.iter().any(|h| h.responses > 0)) {
        match SessionKind::from_metadata(s.session.is_background, s.session.entrypoint.as_deref()) {
            SessionKind::Cli => counts[0] += 1,
            SessionKind::Sdk => counts[1] += 1,
            SessionKind::Background => counts[2] += 1,
            SessionKind::History => {}
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use agent_session_core::model::{HourlyUsage, LedgerSession, SessionUsage};
    use agent_session_core::pricing::{price, price_tokens};
    use chrono::FixedOffset;

    use super::*;

    fn tz() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    fn ms(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis()
    }

    fn day(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    /// 15:53 on Monday 2026-11-16, +8.
    fn now() -> i64 {
        ms("2026-11-16T15:53:00+08:00")
    }

    /// Tokens of one call in these tests.
    const CALL: Tokens = Tokens { input: 1_000, cache_write_5m: 0, cache_write_1h: 2_000, cache_read: 47_000, output: 500, web_search: 0 };

    fn times(n: u64) -> Tokens {
        Tokens {
            input: CALL.input * n,
            cache_write_5m: 0,
            cache_write_1h: CALL.cache_write_1h * n,
            cache_read: CALL.cache_read * n,
            output: CALL.output * n,
            web_search: 0,
        }
    }

    /// What `n` calls of `model` cost.
    fn cost(model: &str, n: u64) -> f64 {
        price_tokens(&price(model).unwrap(), &times(n), false, false).total()
    }

    /// `n` calls of `model` by `session` (of project `/p/<project>`) in the hour starting at `at`.
    struct Calls<'a> {
        session: &'a str,
        project: &'a str,
        at: &'a str,
        model: &'a str,
        n: u64,
    }

    fn ledger(calls: &[Calls]) -> UsageData {
        let mut sessions: std::collections::BTreeMap<String, SessionUsage> = Default::default();
        for c in calls {
            let s = sessions.entry(c.session.to_string()).or_insert_with(|| SessionUsage {
                session: LedgerSession {
                    id: c.session.to_string(),
                    project_key: format!("-p-{}", c.project),
                    project_path: Some(format!("/p/{}", c.project)),
                    title: format!("title of {}", c.session),
                    ..Default::default()
                },
                hours: Vec::new(),
            });
            s.hours.push(HourlyUsage { hour: ms(c.at) / 3_600_000, model: c.model.into(), responses: c.n, tokens: times(c.n), ..Default::default() });
        }
        let mut data = UsageData::default();
        data.replace_all(sessions.into_values().collect());
        data
    }

    fn query(range: UsageRange) -> UsageQuery {
        UsageQuery { range, custom: None, day: None, metric: Metric::Cost, filter: UsageFilter::default() }
    }

    fn build(data: &UsageData, q: &UsageQuery) -> UsageSnapshot {
        UsageSnapshot::build(data, Some(true), q, now(), day("2026-11-16"), &tz())
    }

    const OPUS: &str = "claude-opus-5-5";
    const SONNET: &str = "claude-sonnet-5-5";

    /// Today: 3 Opus calls in s1 (alpha) at 09:00, 1 Sonnet call in s2 (beta) at 15:00.
    /// Yesterday: 2 Opus calls at 10:00 and 2 at 17:00 (after the hour of now).
    fn two_days() -> UsageData {
        ledger(&[
            Calls { session: "s1", project: "alpha", at: "2026-11-16T09:00:00+08:00", model: OPUS, n: 3 },
            Calls { session: "s2", project: "beta", at: "2026-11-16T15:00:00+08:00", model: SONNET, n: 1 },
            Calls { session: "s0", project: "alpha", at: "2026-11-15T10:00:00+08:00", model: OPUS, n: 2 },
            Calls { session: "s0", project: "alpha", at: "2026-11-15T17:00:00+08:00", model: OPUS, n: 2 },
        ])
    }

    #[test]
    fn cards_show_cost_tokens_calls_and_sessions() {
        let snap = build(&two_days(), &query(UsageRange::Today));
        assert_eq!(snap.empty, Empty::No);
        let today = cost(OPUS, 3) + cost(SONNET, 1);
        let [cost_card, input, output, calls, sessions] = &snap.cards[..] else { panic!("five cards: {:?}", snap.cards) };
        assert_eq!((cost_card.label.as_ref(), cost_card.value.as_ref()), ("Cost", fmt_money(today).as_str()));
        // yesterday up to 16:00, the end of the hour under way: the 10:00 calls, not the 17:00 ones
        let before = cost(OPUS, 2);
        let delta = fmt_delta(today, before).unwrap().0;
        assert_eq!(cost_card.sub_text(), format!("yesterday by 16:00 {} · {delta}", fmt_money(before)));
        assert_eq!(cost_card.sub.last().unwrap().1, if today > before { Tone::Up } else { Tone::Down });

        // In is uncached + cache write + cache read, never alone
        assert_eq!((input.label.as_ref(), input.value.as_ref()), ("In", "200k"));
        assert_eq!(input.sub_text(), "cache read 94.0% · cache write 8k · uncached 4k");
        assert_eq!((output.label.as_ref(), output.value.as_ref()), ("Out", "2k"));
        assert_eq!(output.sub_text(), "yesterday by 16:00 1k · ▲ 100.0%");
        assert_eq!((calls.label.as_ref(), calls.value.as_ref()), ("Calls", "4"));
        assert_eq!(calls.sub_text(), format!("{} per call", fmt_money(today / 4.0)));
        assert_eq!((sessions.label.as_ref(), sessions.value.as_ref(), sessions.sub_text().as_str()), ("Sessions", "2", "2 projects"));
    }

    #[test]
    fn the_cost_breakdown_and_the_model_table_add_up_to_the_cost() {
        let snap = build(&two_days(), &query(UsageRange::Today));
        let total = snap.totals.cost.total();
        assert_eq!(snap.totals.tokens.input_total(), snap.totals.tokens.input + snap.totals.tokens.cache_write_5m + snap.totals.tokens.cache_write_1h + snap.totals.tokens.cache_read);
        let names: Vec<&str> = snap.parts.iter().map(|p| p.name.as_ref()).collect();
        assert_eq!(names, ["Cache read", "Cache write", "Output", "Uncached input"], "no web search: no row for it");
        let t = snap.totals.cost;
        let lines = [t.cache_read, t.cache_write_5m + t.cache_write_1h, t.output, t.input];
        for (row, line) in snap.parts.iter().zip(lines) {
            assert_eq!(row.cost.as_ref(), fmt_money(line));
            assert!((row.fraction as f64 - line / total).abs() < 1e-6);
        }
        assert!((lines.iter().sum::<f64>() - total).abs() < 1e-9);
        assert_eq!(snap.parts[0].tokens, "188k");

        let names: Vec<(Option<usize>, &str)> = snap.models.iter().map(|m| (m.slot, m.name.as_ref())).collect();
        assert_eq!(names, [(Some(0), "Opus 5.5"), (Some(1), "Sonnet 5.5"), (None, "Total")], "by cost, then the total");
        let total_row = snap.models.last().unwrap();
        assert_eq!((total_row.calls.as_ref(), total_row.cost.as_ref(), total_row.hit.as_ref()), ("4", fmt_money(total).as_str(), "94.0%"));
        assert_eq!(snap.models[0].share, fmt_pct(cost(OPUS, 3) / total));
    }

    #[test]
    fn the_comparison_reads_no_earlier_data_without_a_previous_span() {
        // nothing the day before yesterday
        let snap = build(&two_days(), &query(UsageRange::Yesterday));
        assert_eq!(snap.cards[0].value, fmt_money(cost(OPUS, 4)), "yesterday counts whole");
        assert_eq!(snap.cards[0].sub_text(), "no earlier data");
        assert_eq!(snap.cards[2].sub_text(), "no earlier data");

        // a day clicked on the chart compares with the whole day before
        let mut q = query(UsageRange::Day);
        q.day = Some(day("2026-11-16"));
        let snap = build(&two_days(), &q);
        assert_eq!(snap.range_label, "Mon Nov 16");
        let delta = fmt_delta(cost(OPUS, 3) + cost(SONNET, 1), cost(OPUS, 4)).unwrap().0;
        assert_eq!(snap.cards[0].sub_text(), format!("day before {} · {delta}", fmt_money(cost(OPUS, 4))));
    }

    #[test]
    fn an_empty_ledger_and_an_empty_range_read_differently() {
        let empty = build(&UsageData::default(), &query(UsageRange::Today));
        assert_eq!(empty.empty, Empty::NoUsage);
        assert!(empty.cards.iter().all(|c| c.value == "—" && c.sub.is_empty()), "{:?}", empty.cards);
        assert_eq!(empty.empty.message(), "No usage recorded yet");
        let not_read = UsageSnapshot::build(&two_days(), None, &query(UsageRange::Today), now(), day("2026-11-16"), &tz());
        assert_eq!(not_read.empty, Empty::NoUsage, "the ledger is not in yet");

        let mut q = query(UsageRange::LastMonth);
        let none = build(&two_days(), &q);
        assert_eq!(none.empty, Empty::NoCalls);
        let values: Vec<&str> = none.cards.iter().map(|c| c.value.as_ref()).collect();
        assert_eq!(values, ["$0.00", "0", "0", "0", "0"]);
        assert!(none.models.is_empty() && none.projects.is_empty() && none.sessions.is_empty());
        assert_eq!(none.empty.message(), "No calls in this range");
        assert_eq!(none.chart.slots.len(), 31, "empty axes: every day of October");

        q.range = UsageRange::Today;
        q.filter.project = Some("-p-nowhere".into());
        assert_eq!(build(&two_days(), &q).empty, Empty::NoCalls, "the filters let nothing through");

        let gone = UsageSnapshot::build(&UsageData::default(), Some(false), &query(UsageRange::Today), now(), day("2026-11-16"), &tz());
        assert_eq!(gone.empty, Empty::Unavailable);
        assert_eq!(gone.empty.message(), "Usage ledger unavailable: it was written by a newer version of the app");
    }

    #[test]
    fn unpriced_calls_are_called_out_on_the_cost_card() {
        let data = ledger(&[
            Calls { session: "s1", project: "alpha", at: "2026-11-16T09:00:00+08:00", model: OPUS, n: 1 },
            Calls { session: "s1", project: "alpha", at: "2026-11-16T10:00:00+08:00", model: "claude-unknown-9", n: 3 },
        ]);
        let snap = build(&data, &query(UsageRange::Today));
        assert_eq!(snap.cards[0].value, fmt_money(cost(OPUS, 1)), "the unpriced calls cost nothing");
        assert_eq!(snap.cards[0].sub.last().unwrap(), &(SharedString::from(" · 3 calls unpriced"), Tone::Warn));
        assert_eq!(snap.cards[3].value, "4", "but they are calls");
        assert_eq!(snap.cards[1].value, "200k", "and their tokens count");
        let unknown = snap.models.iter().find(|m| m.name == display_name("claude-unknown-9")).expect("a row of its own");
        assert_eq!((unknown.slot, unknown.cost.as_ref(), unknown.calls.as_ref()), (Some(4), "—", "3"));
    }

    #[test]
    fn the_project_table_folds_the_tail_into_other_projects() {
        let projects = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
        let calls: Vec<Calls> = projects
            .iter()
            .enumerate()
            .map(|(ix, p)| Calls { session: p, project: p, at: "2026-11-16T09:00:00+08:00", model: OPUS, n: (projects.len() - ix) as u64 })
            .collect();
        let data = ledger(&calls);
        let snap = build(&data, &query(UsageRange::Today));
        let names: Vec<&str> = snap.projects.iter().map(|p| p.name.as_ref()).collect();
        assert_eq!(names, ["a", "b", "c", "d", "e", "f", "g", "2 other projects"]);
        let other = snap.projects.last().unwrap();
        assert_eq!((other.key.as_ref(), other.cost.as_ref(), other.sessions.as_ref()), (None, fmt_money(cost(OPUS, 2) + cost(OPUS, 1)).as_str(), "2"));
        assert_eq!(snap.projects[0].fraction, 1.0);
        assert!((snap.projects[1].fraction - 8.0 / 9.0).abs() < 1e-6, "bars against the first row");
        assert!(snap.projects.iter().all(|p| !p.selected));

        let mut q = query(UsageRange::Today);
        q.filter.project = Some("-p-c".into());
        let picked = build(&data, &q);
        assert_eq!(picked.projects.len(), 1, "the filter leaves only its project");
        assert!(picked.projects[0].selected);
        assert_eq!(picked.cards[0].value, picked.projects[0].cost, "Cost is the project's cost");
    }

    #[test]
    fn sessions_are_listed_by_their_cost_inside_the_range() {
        let data = ledger(&[
            Calls { session: "11111111-aaaa", project: "alpha", at: "2026-11-15T09:00:00+08:00", model: OPUS, n: 50 },
            Calls { session: "11111111-aaaa", project: "alpha", at: "2026-11-16T09:00:00+08:00", model: OPUS, n: 1 },
            Calls { session: "22222222-bbbb", project: "beta", at: "2026-11-16T09:00:00+08:00", model: OPUS, n: 2 },
        ]);
        let snap = build(&data, &query(UsageRange::Today));
        let rows: Vec<(&str, &str, &str, &str)> = snap.sessions.iter().map(|s| (s.short_id.as_ref(), s.title.as_ref(), s.project.as_ref(), s.cost.as_ref())).collect();
        let (one, two) = (fmt_money(cost(OPUS, 1)), fmt_money(cost(OPUS, 2)));
        assert_eq!(rows, [("22222222", "title of 22222222-bbbb", "beta", two.as_str()), ("11111111", "title of 11111111-aaaa", "alpha", one.as_str())]);
        assert_eq!(snap.sessions[0].input, "100k");
    }

    #[test]
    fn a_single_day_charts_hours_and_several_days_chart_days() {
        let data = two_days();
        let today = build(&data, &query(UsageRange::Today));
        let chart = &today.chart;
        assert!(chart.hourly);
        assert_eq!(chart.title, "By hour");
        assert_eq!(chart.slots.len(), 24);
        let labels: Vec<&str> = chart.slots.iter().map(|s| s.label.as_ref()).collect();
        assert_eq!((labels[0], labels[5], labels[23]), ("04", "09", "03"), "from 04 to 03 the next morning");
        assert_eq!(chart.slots[5].totals.responses, 3);
        assert_eq!(chart.slots[5].by_model[0].responses, 3, "Opus is the first color");
        assert_eq!(chart.slots[11].by_model[1].responses, 1, "Sonnet the second");
        assert_eq!(chart.slots[6].ghost.unwrap().responses, 2, "10:00 yesterday behind 10:00 today");
        assert!(!chart.slots[11].future && chart.slots[12].future, "16:00 is still to come");
        let (at, label) = chart.now.clone().unwrap();
        assert!((at - (11.0 + 53.0 / 60.0)).abs() < 1e-9);
        assert_eq!(label, "15:53");
        assert_eq!(chart.legend, [Legend::Model(0, "Opus 5.5".into()), Legend::Model(1, "Sonnet 5.5".into()), Legend::Ghost]);
        let stacked: f64 = chart.slots[5].stack(Metric::Cost).iter().sum();
        assert!((stacked - chart.slots[5].value(Metric::Cost).unwrap()).abs() < 1e-9, "the segments make up the bar");
        assert!(chart.y_max >= cost(OPUS, 3) && chart.ticks.first().unwrap().0 == 0.0 && chart.ticks.last().unwrap().0 == chart.y_max);

        let mut q = query(UsageRange::Days7);
        q.metric = Metric::Out;
        let week = build(&data, &q);
        let chart = &week.chart;
        assert!(!chart.hourly);
        assert_eq!(chart.title, "By day");
        let labels: Vec<&str> = chart.slots.iter().map(|s| s.label.as_ref()).collect();
        assert_eq!(labels, ["11-10", "11-11", "11-12", "11-13", "11-14", "11-15", "Today"], "seven usage days, today included");
        assert!(chart.slots[6].partial && !chart.slots[5].partial);
        assert_eq!(chart.slots[5].day, day("2026-11-15"));
        assert!(chart.slots[4].weekend && chart.slots[5].weekend && !chart.slots[6].weekend, "Nov 14 and 15 are a weekend");
        // the average takes the day and the six before it, inside the range or not
        assert_eq!(chart.slots[5].average, Some(2_000.0 / 7.0));
        assert_eq!(chart.slots[6].average, Some(4_000.0 / 7.0));
        assert_eq!(chart.legend, [Legend::Model(0, "Opus 5.5".into()), Legend::Model(1, "Sonnet 5.5".into()), Legend::Average]);
        assert_eq!(chart.now, None);

        // a Custom range of one day is charted by the hour; the hit rate is a line from below 100%
        let mut q = query(UsageRange::Custom);
        q.custom = Some((day("2026-11-15"), day("2026-11-15")));
        q.metric = Metric::HitRate;
        let one = build(&data, &q);
        assert!(one.chart.hourly && one.chart.now.is_none());
        assert_eq!(one.chart.legend, [Legend::HitRate]);
        assert_eq!(one.chart.slots[6].value(Metric::HitRate), Some(0.94));
        assert!(one.chart.y_min > 0.0 && one.chart.y_min <= 0.94 && one.chart.y_max == 1.0);
        assert_eq!(one.chart.slots[0].value(Metric::HitRate), None, "no In, no point");
    }

    #[test]
    fn a_range_s_bounds_follow_the_four_o_clock_days() {
        let data = two_days();
        let at = |s: &str| ms(s);
        assert_eq!(build(&data, &query(UsageRange::Today)).span, Some(Span { from_ms: at("2026-11-16T04:00:00+08:00"), until_ms: at("2026-11-17T04:00:00+08:00") }));
        let mut q = query(UsageRange::Custom);
        q.custom = Some((day("2026-11-02"), day("2026-11-08")));
        let custom = build(&data, &q);
        assert_eq!(custom.span, Some(Span { from_ms: at("2026-11-02T04:00:00+08:00"), until_ms: at("2026-11-09T04:00:00+08:00") }), "the last day included");
        assert_eq!(custom.range_label, "Nov 2 – Nov 8");
        assert_eq!(custom.chart.slots.len(), 7);
        assert_eq!(build(&data, &query(UsageRange::Days30)).range_label, "Last 30 days");
    }
}
