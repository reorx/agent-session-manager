//! The Usage view: what the calls in a time range cost and how many tokens they took, by hour or
//! by day, by kind of token, model, project and session.
//!
//! Everything on screen comes from one `UsageSnapshot`, built by a pure function from the
//! in-memory usage data (`UsageData`, from the ledger), the range, the filters and the clock. It
//! is built again only when one of those changed (`AppRoot::refresh_usage`); render only reads its
//! strings. The chart (`usage_chart.rs`) draws `UsageSnapshot::chart`.

use std::cell::Cell;
use std::rc::Rc;
use std::path::PathBuf;
use std::time::{Duration as StdDuration, Instant};

use agent_session_core::format::fmt_tokens;
use agent_session_core::model::Tokens;
use agent_session_core::pricing::display_name;
use agent_session_core::usage::{
    Metric, ProjectEntry, SlotTotals, Span, Totals, UsageData, UsageFilter, UsageRange, day_slots, day_start_ms, hour_slots, model_slot,
    moving_average, previous_span, range_span, usage_day,
};
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Weekday};
use gpui_kit::assets::IconName;
use gpui_kit::component::date_picker::DatePickerState;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::*;

use crate::AppRoot;
use crate::model::{Filter, MainView};
use crate::resident::thousands;
use crate::settings::ViewState;
use crate::style::{self, FluentBuilder};
use crate::toolbar::{segment, segmented};
use crate::usage_chart::UsageChart;

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
    /// A chart bar whose tooltip shows without the mouse (autoplay's hover step); the mouse
    /// brings its own tooltip (`usage_chart.rs`).
    pub hover: Option<usize>,
    /// Where the chart was last painted: a click in the chart area finds its bar from it.
    pub chart_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The Project dropdown: every project with calls in the ledger, transcripts gone or not.
    pub projects: Vec<ProjectEntry>,
    /// The Model dropdown: every model in the ledger and its calls, most calls first.
    pub models: Vec<(String, u64)>,
    /// Sessions with calls by kind (CLI, SDK, BG), for the Kind dropdown.
    pub kind_counts: [usize; 3],
    /// The toolbar's line between the ranges and Project / Kind / Model is painted: it sat
    /// between two controls on one row when last laid out (a wrapped toolbar would end a row
    /// with it).
    pub separator_shown: Rc<Cell<bool>>,
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
            chart_bounds: Rc::default(),
            projects: Vec::new(),
            models: Vec::new(),
            kind_counts: [0; 3],
            separator_shown: Rc::new(Cell::new(true)),
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
    /// The session table has a row for it (`AppRoot::refresh_usage` checks); without one its
    /// transcript and its history are gone, and a click can only say so.
    pub listed: bool,
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

/// A value of `metric` as the chart's tooltip and labels write it.
pub fn fmt_metric(metric: Metric, value: f64) -> String {
    match metric {
        Metric::Cost => fmt_money(value),
        Metric::In | Metric::Out => fmt_tokens(value.round() as u64),
        Metric::HitRate => fmt_pct(value),
    }
}

/// The label of a value axis line `step` apart from the next: `$300`, `$1.5k`, `$0.25`,
/// `$0.005`, `120M`, `1.5k`, `96%`, `99.25%`. As many decimals as the step needs, so no two
/// lines read alike (a day of Haiku is a fraction of a cent).
pub fn fmt_axis(metric: Metric, value: f64, step: f64) -> String {
    match metric {
        Metric::Cost if value == 0.0 => "$0".into(),
        Metric::Cost if value >= 1000.0 => {
            let (unit, suffix) = unit_of(value);
            format!("${}{suffix}", axis_number(value, step, unit))
        }
        // cents keep both digits: $2.50, not $2.5
        Metric::Cost => match decimals(step) {
            0 => format!("${value:.0}"),
            d => format!("${value:.*}", d.max(2)),
        },
        Metric::In | Metric::Out => {
            let (unit, suffix) = unit_of(value);
            format!("{}{suffix}", axis_number(value, step, unit))
        }
        Metric::HitRate => format!("{}%", axis_number(value * 100.0, step * 100.0, 1.0)),
    }
}

/// `value` in `unit` with the decimals the step needs there, trailing zeros dropped: `1.5`, `2`.
fn axis_number(value: f64, step: f64, unit: f64) -> String {
    let text = format!("{:.*}", decimals(step / unit), value / unit);
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
}

/// The fewest decimals that write `x` exactly (at most 8).
fn decimals(x: f64) -> usize {
    (0..8)
        .find(|d| {
            let scaled = x * 10f64.powi(*d as i32);
            (scaled - scaled.round()).abs() < 1e-6 * scaled.abs().max(1.0)
        })
        .unwrap_or(8)
}

/// The unit a number is written in: as is, thousands, millions, billions.
fn unit_of(value: f64) -> (f64, &'static str) {
    if value >= 1e9 {
        (1e9, "B")
    } else if value >= 1e6 {
        (1e6, "M")
    } else if value >= 1e3 {
        (1e3, "k")
    } else {
        (1.0, "")
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

/// `Mon Nov 9`, and `Wed Dec 31, 2025` outside `year` (this year).
pub fn fmt_day_in(day: NaiveDate, year: i32) -> String {
    if day.year() == year { fmt_day(day) } else { format!("{}, {}", fmt_day(day), day.year()) }
}

impl UsageSnapshot {
    /// Everything the view shows for `q`, at `now_ms` on the usage day `today` in `tz`.
    /// `available`: `None` until the store said whether this run has a ledger.
    pub fn build<T: TimeZone>(data: &UsageData, available: Option<bool>, q: &UsageQuery, now_ms: i64, today: NaiveDate, tz: &T) -> Self {
        let span = range_span(q.range, today, q.custom, q.day, tz);
        let range_label = range_label(q, span, today.year(), tz).into();
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
        listed: true,
    }
}

fn local<T: TimeZone>(ms: i64, tz: &T) -> NaiveDateTime {
    tz.timestamp_millis_opt(ms).single().expect("valid timestamp").naive_local()
}

/// The span's last usage day (its end is the start of the day after).
fn last_day<T: TimeZone>(span: Span, tz: &T) -> NaiveDate {
    usage_day(span.until_ms - 1, tz)
}

/// What the range is called; a day or a Custom range outside `year` (this year) says its year.
fn range_label<T: TimeZone>(q: &UsageQuery, span: Span, year: i32, tz: &T) -> String {
    let (first, last) = (usage_day(span.from_ms, tz), last_day(span, tz));
    match q.range {
        UsageRange::Today => "Today".into(),
        UsageRange::Yesterday => "Yesterday".into(),
        UsageRange::Days7 => "Last 7 days".into(),
        UsageRange::Days30 => "Last 30 days".into(),
        UsageRange::ThisMonth => "This month".into(),
        UsageRange::LastMonth => "Last month".into(),
        UsageRange::Custom if first == last => fmt_day_in(first, year),
        UsageRange::Custom if first.year() != last.year() => format!("{} – {}", first.format("%b %-d, %Y"), last.format("%b %-d, %Y")),
        UsageRange::Custom if first.year() != year => format!("{} – {}", first.format("%b %-d"), last.format("%b %-d, %Y")),
        UsageRange::Custom => format!("{} – {}", first.format("%b %-d"), last.format("%b %-d")),
        UsageRange::Day => fmt_day_in(first, year),
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
        // the dashed box and line only when there is one to see
        let ghost = slots.iter().any(|s| s.ghost.is_some_and(|g| g.responses > 0));
        let average = slots.iter().any(|s| s.average.is_some_and(|a| a > 0.0));
        if hourly && ghost {
            legend.push(Legend::Ghost);
        } else if !hourly && average {
            legend.push(Legend::Average);
        }
    }
    let now = (hourly && first == today)
        .then(|| slots.iter().position(|s| s.span.from_ms <= now_ms && now_ms < s.span.until_ms))
        .flatten()
        .map(|ix| {
            let s = slots[ix].span;
            (ix as f64 + (now_ms - s.from_ms) as f64 / (s.until_ms - s.from_ms) as f64, local(now_ms, tz).format("%H:%M").to_string().into())
        });
    let label = range_label(q, span, today.year(), tz);
    let subtitle = if !hourly {
        format!("{label} · click a bar to see its hours")
    } else if matches!(q.range, UsageRange::Today | UsageRange::Yesterday) {
        format!("{label}, {}", fmt_day_in(first, today.year()))
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

/// The hours of `day`, each with the same clock hour of the day before behind it.
fn hour_chart<T: TimeZone>(data: &UsageData, q: &UsageQuery, day: NaiveDate, now_ms: i64, today: NaiveDate, tz: &T) -> Vec<ChartSlot> {
    let spans = hour_slots(day, tz);
    let before_spans = hour_slots(day - Duration::days(1), tz);
    let before = data.series(&before_spans, &q.filter);
    let series = data.series(&spans, &q.filter);
    let facing = same_clock_hours(&spans, &before_spans, tz);
    spans
        .iter()
        .zip(series)
        .zip(facing)
        .map(|((span, t), facing)| {
            let mut slot = slot_of(*span, local(span.from_ms, tz).format("%H").to_string(), day, t);
            slot.ghost = facing.map(|ix| before[ix].totals);
            slot.future = day == today && span.from_ms > now_ms;
            slot
        })
        .collect()
}

/// For each of `hours`, the hour of `before` (the day before) that starts at the same clock
/// time. A day with a DST change has 23 or 25 hours, so the n-th hour of one day is not the
/// n-th of the other: the k-th time a clock time comes round faces its k-th time the day before
/// (the night the clocks go back has two 01:00s), and an hour the day before did not have (the
/// clocks went forward) faces none.
fn same_clock_hours<T: TimeZone>(hours: &[Span], before: &[Span], tz: &T) -> Vec<Option<usize>> {
    let clock = |spans: &[Span], shift: Duration| -> Vec<(NaiveDateTime, usize)> {
        let mut seen: std::collections::HashMap<NaiveDateTime, usize> = Default::default();
        spans
            .iter()
            .map(|s| {
                let at = local(s.from_ms, tz) + shift;
                let k = seen.entry(at).or_default();
                *k += 1;
                (at, *k)
            })
            .collect()
    };
    let earlier: std::collections::HashMap<(NaiveDateTime, usize), usize> = clock(before, Duration::days(1)).into_iter().enumerate().map(|(ix, at)| (at, ix)).collect();
    clock(hours, Duration::zero()).iter().map(|at| earlier.get(at).copied()).collect()
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
        let step = nice_step(1.0 - floor, 4, true);
        return ((floor / step).floor() * step, 1.0, step);
    }
    let highest = slots
        .iter()
        .flat_map(|s| [s.value(metric), s.average, s.ghost.and_then(|g| metric_value(&g, metric))])
        .flatten()
        .fold(0.0, f64::max);
    let top = if highest > 0.0 { highest } else { 1.0 };
    // tokens are counted whole, and in steps that read whole in k / M (no 2.5k)
    let tokens = matches!(metric, Metric::In | Metric::Out);
    let step = if tokens { nice_step(top, 4, false).max(1.0) } else { nice_step(top, 4, true) };
    (0.0, (top / step).ceil() * step, step)
}

/// A round step (1, 2, 5 × 10ⁿ, and 2.5 × 10ⁿ with `quarters`) that splits `span` into about
/// `parts`.
fn nice_step(span: f64, parts: u32, quarters: bool) -> f64 {
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
    } else if f <= 2.5 && quarters {
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
        let mut snapshot = UsageSnapshot::build(&self.usage, self.usage_available, &query, crate::now_ms(), self.usage_day, &Local);
        let delegate = self.table.read(cx).delegate();
        for line in &mut snapshot.sessions {
            line.listed = delegate.row_by_id(&line.id).is_some();
        }
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
/// The column of the project table's `↗`.
const JUMP_W: f32 = 28.;
/// The tooltip of a session's grey id: the session table has no row for it.
pub const UNLISTED_TIP: &str = "Transcript cleaned up: only its usage is kept";
/// The first line of "Most expensive sessions", the first project's `↗` and the Sessions card's
/// (tests click them).
pub const SESSION_LINE_SELECTOR: &str = "usage-session-line-0";
pub const PROJECT_JUMP_SELECTOR: &str = "usage-project-jump-0";
pub const SESSIONS_JUMP_SELECTOR: &str = "usage-sessions-jump";
/// The five cards and their big numbers (tests).
pub const CARD_SELECTORS: [&str; 5] = ["usage-card-0", "usage-card-1", "usage-card-2", "usage-card-3", "usage-card-4"];
pub const CARD_VALUE_SELECTORS: [&str; 5] = ["usage-card-value-0", "usage-card-value-1", "usage-card-value-2", "usage-card-value-3", "usage-card-value-4"];

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
                    .child(render_cards(&snap.cards, narrow, self.render_sessions_jump(cx), cx))
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
        let area = div().id(CHART_ID).w_full().h(px(CHART_H)).flex_none();
        let area = if no_usage {
            area.flex().items_center().justify_center().text_size(px(12.5)).text_color(style::muted()).child(Empty::NoUsage.message())
        } else {
            // a click on a day's bar opens its hours (`Plot` has no click of its own)
            area.child(div().size_full().font_family(style::mono(cx)).child(UsageChart::new(chart.clone(), u.chart_bounds.clone(), u.hover)))
                .when(!chart.hourly, |el| el.cursor_pointer())
                .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    this.click_chart(e.position, cx);
                }))
        };
        panel()
            .border_b_1()
            .border_color(style::hairline())
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
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

    /// The Sessions card's `↗`: the sessions active in the range, in the session table. Only
    /// when there are some.
    fn render_sessions_jump(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        (self.usage_view.snapshot.empty == Empty::No).then(|| {
            jump_button("usage-sessions-jump", "These sessions in the session table")
                .debug_selector(|| SESSIONS_JUMP_SELECTOR.into())
                .on_click(cx.listener(|this, _, window, cx| this.show_active_sessions(None, window, cx)))
        })
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
                // the project's sessions active in the range, in the session table; its own click,
                // not also the row's (which filters Usage by the project)
                let arrow = r.key.clone().map(|key| {
                    jump_button(("usage-project-jump", ix), "Its sessions active in this range, in the session table")
                        .when(ix == 0, |el| el.debug_selector(|| PROJECT_JUMP_SELECTOR.into()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.show_active_sessions(Some(key.clone()), window, cx);
                        }))
                });
                let row = table_row(false, r.selected)
                    .child(name_cell(div().truncate().when(r.key.is_none(), |el| el.text_color(style::muted())).child(r.name.clone())))
                    .child(div().w(px(100.)).flex_none().flex().justify_end().child(track))
                    .child(num_cell(r.cost.clone(), cx))
                    .child(num_cell(r.share.clone(), cx))
                    .child(num_cell(r.sessions.clone(), cx))
                    .child(div().w(px(JUMP_W)).flex_none().flex().justify_end().children(arrow));
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
            .child(panel_head("By project", "click a project to filter by it, ↗ for its sessions"))
            .child(header_row("Project", &["", "Cost", "Share", "Sessions"]).child(div().w(px(JUMP_W)).flex_none()))
            .children(rows)
            .when(snap.projects.is_empty(), |el| el.child(empty_note(snap.empty)))
    }

    /// The sessions that cost the most inside the range; a click on one shows it in the session
    /// table. One without a row there has a grey id that says why.
    fn render_sessions(&self, cx: &mut Context<Self>) -> Div {
        let snap = &self.usage_view.snapshot;
        let rows: Vec<Stateful<Div>> = snap.sessions.iter().enumerate().map(|(ix, s)| {
            let short_id = div()
                .id(("usage-session-id", ix))
                .flex_none()
                .font_family(style::mono(cx))
                .text_size(px(11.5))
                .text_color(if s.listed { style::accent() } else { style::muted() })
                .child(s.short_id.clone())
                .when(!s.listed, |el| el.tooltip(|window, cx| Tooltip::new(UNLISTED_TIP).build(window, cx)));
            let cell = h_flex()
                .gap(px(8.))
                .min_w_0()
                .child(short_id)
                .child(div().min_w_0().truncate().child(s.title.clone()))
                .child(div().flex_none().max_w(px(140.)).truncate().text_size(px(11.5)).text_color(style::muted()).child(s.project.clone()));
            let id = s.id.clone();
            div()
                .id(("usage-session-row", ix))
                .cursor_pointer()
                .hover(|s| s.bg(style::hover_bg()))
                .when(ix == 0, |el| el.debug_selector(|| SESSION_LINE_SELECTOR.into()))
                .child(table_row(false, false).child(name_cell(cell)).child(num_cell(s.input.clone(), cx)).child(num_cell(s.cost.clone(), cx)))
                .on_click(cx.listener(move |this, _, window, cx| this.reveal_session(&id, window, cx)))
        }).collect();
        panel()
            .border_t_1()
            .border_color(style::hairline())
            .child(panel_head("Most expensive sessions", "cost inside the range, click one to see it"))
            .child(header_row("Session", &["In", "Cost"]))
            .children(rows)
            .when(snap.sessions.is_empty(), |el| el.child(empty_note(snap.empty)))
    }
}

/// The five cards in a row, or when the window is narrow three over two (five cards of a
/// 700 px window had no room for a number like `$5,913.96`); `jump` goes next to the Sessions
/// card's label. A number never breaks.
fn render_cards(cards: &[Card], narrow: bool, mut jump: Option<Stateful<Div>>, cx: &App) -> Div {
    let per_row = if narrow { 3 } else { cards.len() };
    let mut rows: Vec<Div> = Vec::new();
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
        if ix % per_row == 0 {
            rows.push(h_flex().w_full().items_stretch().when(ix > 0, |el| el.border_t_1().border_color(style::hairline())));
        }
        let row = rows.pop().expect("a row");
        rows.push(row.child(
            v_flex()
                .debug_selector(move || CARD_SELECTORS[ix].into())
                .flex_1()
                .min_w_0()
                .px(px(20.))
                .pt(px(16.))
                .pb(px(15.))
                .gap(px(4.))
                .when(ix % per_row > 0, |el| el.border_l_1().border_color(style::hairline()))
                .child(
                    h_flex()
                        .h(px(16.))
                        .justify_between()
                        .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(style::label()).child(card.label.to_uppercase()))
                        .when(card.label == "Sessions", |el| el.children(jump.take())),
                )
                .child(
                    div()
                        .debug_selector(move || CARD_VALUE_SELECTORS[ix].into())
                        // as wide as the number (not the card), so tests see it fit
                        .self_start()
                        .whitespace_nowrap()
                        .font_family(style::mono(cx))
                        .text_size(px(24.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_height(px(30.))
                        .child(card.value.clone()),
                )
                .child(div().w_full().text_size(px(12.)).min_h(px(16.)).text_color(style::text_secondary()).child(sub)),
        ));
    }
    v_flex().w_full().border_b_1().border_color(style::hairline()).children(rows)
}

fn render_legend(legend: &[Legend]) -> Div {
    let mut row = h_flex().gap(px(12.)).text_size(px(11.5)).text_color(style::text_secondary());
    for entry in legend {
        let (mark, name): (Div, SharedString) = match entry {
            Legend::Model(slot, name) => (swatch(style::model_color(*slot)), name.clone()),
            Legend::Ghost => (div().size(px(9.)).rounded(px(2.)).border_1().border_dashed().border_color(style::chart_ghost()), "day before".into()),
            Legend::Average => (div().w(px(14.)).h(px(0.)).border_t_2().border_dashed().border_color(style::chart_average()), "7-day average".into()),
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

/// A small `↗` button: to the session table.
fn jump_button(id: impl Into<ElementId>, tooltip: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|s| s.bg(style::control_hover()))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(Icon::new(IconName::ArrowUpRight).size(px(13.)).text_color(style::accent()))
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

/// The steps of the Usage view's autoplay (`ASM_USAGE_VIEW_AUTOPLAY`), in order.
pub const AUTOPLAY_STEPS: [&str; 17] = [
    "today",
    "today-hover",
    "yesterday",
    "d30",
    "d30-in",
    "d30-hit",
    "d30-hover",
    "drill",
    "back",
    "project",
    "month",
    "narrow",
    "sessions",
    "jump-project",
    "jump-session",
    "sessions-narrow",
    "empty",
];
/// The window width of the autoplay's `narrow` step: under `NARROW_W`, so the panels stack.
const AUTOPLAY_NARROW_W: f32 = 900.;
/// The window width of `sessions-narrow`: the session table's toolbar takes more than one row.
const AUTOPLAY_SESSIONS_NARROW_W: f32 = 700.;

impl AppRoot {
    /// Scripted tour of the Usage view for screenshots (`make screenshot-usage-view`). Once the
    /// start-up scan and the ledger are in, it runs `AUTOPLAY_STEPS` (or the ones named in
    /// `ASM_USAGE_VIEW_STEPS`, comma-separated) and prints `autoplay-usage-view: <step> · <state>`
    /// after each has been drawn (`<step> skipped · <why>` when it cannot be done, `done` at the
    /// end). Then it holds that view until the script drops `<step>.shot` into `gate` (at most
    /// 30 s), so a slow capture never sees the next step:
    /// - `today` / `yesterday` / `d30` / `month`: Usage, that range, the chart in Cost;
    /// - `today-hover` / `d30-hover`: the tooltip of the highest bar (by the hour: of now, when
    ///   today has no calls yet) shown without the mouse (`UsageState::hover`), in Cost;
    /// - `d30-in` / `d30-hit`: the chart in In / Hit rate;
    /// - `drill` / `back`: into the most expensive of the 30 days, and out again (Escape's path);
    /// - `project`: the Project filter set to the most expensive project of the 30 days;
    /// - `narrow`: the window 900 px wide (the panels stack), its width restored after the capture;
    /// - `sessions`: the session table, the project still set;
    /// - `jump-project`: from Usage, the most expensive project's `↗` (`show_active_sessions`):
    ///   the table with that project, Updated any time and the Active filter; the rows listed go to
    ///   the log as `jump-rows: <id>,<id>,…`;
    /// - `jump-session`: from Usage, the most expensive session the table has a row for, as a
    ///   click on it does (`reveal_session`): selected, in the detail panel, the keys on the table;
    /// - `sessions-narrow`: the session table in a window 700 px wide, the Active tag still on:
    ///   its toolbar wraps (the state's `toolbar=<rows>/<inside|overflow>`), its width restored
    ///   after the capture;
    /// - `empty`: Usage, no project, a Custom range in 2000 (no calls).
    pub(crate) fn usage_view_autoplay(&self, gate: PathBuf, window: &Window, cx: &mut Context<Self>) -> Task<()> {
        let only: Option<Vec<String>> = std::env::var("ASM_USAGE_VIEW_STEPS").ok().filter(|v| !v.trim().is_empty()).map(|v| v.split(',').map(|s| s.trim().to_string()).collect());
        let settled = |this: &AppRoot| !this.scan_in_flight && !this.today_label.is_empty() && matches!(this.index_status, crate::IndexStatus::Idle(_));
        cx.spawn_in(window, async move |this, cx| {
            if !crate::autoplay_wait(&this, cx, 1200, |this, _| settled(this)).await {
                println!("autoplay-usage-view: never settled");
                return;
            }
            for step in AUTOPLAY_STEPS.iter().filter(|s| only.as_ref().is_none_or(|only| only.iter().any(|o| o == *s))) {
                let Ok(done) = this.update_in(cx, |this, window, cx| this.usage_autoplay_step(step, window, cx)) else { return };
                let restore = match done {
                    Ok(restore) => restore,
                    Err(why) => {
                        println!("autoplay-usage-view: {step} skipped · {why}");
                        continue;
                    }
                };
                // the scan that showing the view starts, then a frame or two
                crate::autoplay_wait(&this, cx, 300, |this, _| settled(this)).await;
                cx.background_executor().timer(StdDuration::from_millis(700)).await;
                let Ok(state) = this.update_in(cx, |this, window, cx| this.usage_autoplay_state(window, cx)) else { return };
                println!("autoplay-usage-view: {step} · {state}");
                let shot = gate.join(format!("{step}.shot"));
                for _ in 0..300 {
                    if shot.exists() {
                        break;
                    }
                    cx.background_executor().timer(StdDuration::from_millis(100)).await;
                }
                if let Some(size) = restore {
                    this.update_in(cx, |_, window, _| window.resize(size)).ok();
                    cx.background_executor().timer(StdDuration::from_millis(300)).await;
                }
            }
            println!("autoplay-usage-view: done");
        })
    }

    /// One step of `usage_view_autoplay`; `Ok(Some(size))` when the window is to get that size
    /// back after the capture.
    fn usage_autoplay_step(&mut self, step: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<Option<Size<Pixels>>, String> {
        let range = |this: &mut Self, range: UsageRange, cx: &mut Context<Self>| {
            this.usage_view.hover = None;
            if this.usage_view.metric != Metric::Cost {
                this.set_usage_metric(Metric::Cost, cx);
            }
            this.set_usage_range(range, cx);
        };
        match step {
            "today" => {
                self.set_view(MainView::Usage, window, cx);
                range(self, UsageRange::Today, cx);
            }
            "yesterday" => range(self, UsageRange::Yesterday, cx),
            "d30" => range(self, UsageRange::Days30, cx),
            "month" => range(self, UsageRange::ThisMonth, cx),
            "d30-in" => self.set_usage_metric(Metric::In, cx),
            "d30-hit" => self.set_usage_metric(Metric::HitRate, cx),
            "today-hover" | "d30-hover" => {
                self.set_usage_metric(Metric::Cost, cx);
                let chart = &self.usage_view.snapshot.chart;
                let highest = chart
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.totals.responses > 0)
                    .max_by(|a, b| a.1.totals.cost.total().total_cmp(&b.1.totals.cost.total()))
                    .map(|(ix, _)| ix);
                let ix = highest.or_else(|| chart.now.as_ref().map(|(at, _)| *at as usize)).ok_or("no bar with calls")?;
                self.usage_view.hover = Some(ix);
                cx.notify();
            }
            "drill" => {
                let chart = &self.usage_view.snapshot.chart;
                let day = chart.slots.iter().filter(|s| s.totals.responses > 0).max_by(|a, b| a.totals.cost.total().total_cmp(&b.totals.cost.total())).map(|s| s.day);
                let day = day.ok_or("no day with calls")?;
                if !self.drill_into_day(day, cx) {
                    return Err(format!("{day} did not open"));
                }
            }
            "back" => self.leave_day(cx),
            "project" => {
                let key = self.usage_view.snapshot.projects.first().and_then(|p| p.key.clone()).ok_or("no project with calls")?;
                self.set_project(Some(key), cx);
            }
            "narrow" => {
                let size = window.viewport_size();
                window.resize(gpui_kit::size(px(AUTOPLAY_NARROW_W), size.height));
                return Ok(Some(size));
            }
            "sessions" => self.set_view(MainView::Sessions, window, cx),
            "jump-project" => {
                self.set_view(MainView::Usage, window, cx);
                let key = self.usage_view.snapshot.projects.first().and_then(|p| p.key.clone()).ok_or("no project with calls")?;
                self.show_active_sessions(Some(key), window, cx);
                let delegate = self.table.read(cx).delegate();
                let ids: Vec<&str> = delegate.ix.iter().map(|i| delegate.rows[*i].id.as_ref()).collect();
                println!("jump-rows: {}", ids.join(","));
            }
            "jump-session" => {
                self.set_view(MainView::Usage, window, cx);
                let id = self.usage_view.snapshot.sessions.iter().find(|s| s.listed).map(|s| s.id.clone()).ok_or("no session of the range in the table")?;
                self.reveal_session(&id, window, cx);
            }
            "sessions-narrow" => {
                self.set_view(MainView::Sessions, window, cx);
                let size = window.viewport_size();
                window.resize(gpui_kit::size(px(AUTOPLAY_SESSIONS_NARROW_W), size.height));
                return Ok(Some(size));
            }
            "empty" => {
                self.set_view(MainView::Usage, window, cx);
                self.set_project(None, cx);
                let (first, last) = (NaiveDate::from_ymd_opt(2000, 1, 3).expect("a date"), NaiveDate::from_ymd_opt(2000, 1, 9).expect("a date"));
                // as a pick in the date picker would, which then shows it
                self.usage_view.date_picker.update(cx, |picker, cx| picker.set_date((first, last), window, cx));
                self.set_usage_custom_range(first, last, cx);
            }
            other => return Err(format!("unknown step {other}")),
        }
        Ok(None)
    }

    /// What the autoplay prints after a step: `view=usage range=Last 30 days metric=cost
    /// chart=daily/30 hover=12 empty=no calls=1234 cost=$… project=<key> width=1440 today=Today $…
    /// today-matches=yes`, then the session table's side: `active=2026-11-01..2026-11-16
    /// updated=any|limited rows=12 selected=<short id> detail=yes focus=table|search|usage|-
    /// usage-sessions=14` (Usage's Sessions card) `toolbar=2/inside` (rows of the session table's
    /// toolbar and whether all its controls are in the window; `-` in Usage). The script checks
    /// these.
    fn usage_autoplay_state(&self, window: &Window, cx: &App) -> String {
        let u = &self.usage_view;
        let snap = &u.snapshot;
        let chart = &snap.chart;
        let empty = match snap.empty {
            Empty::No => "no",
            Empty::NoUsage => "no-usage",
            Empty::NoCalls => "no-calls",
            Empty::Unavailable => "unavailable",
        };
        let metric = match u.metric {
            Metric::Cost => "cost",
            Metric::In => "in",
            Metric::Out => "out",
            Metric::HitRate => "hit",
        };
        // Today's cost card against the status bar's Today: the same number (acceptance 3)
        let today_matches = match (u.range, self.usage_available) {
            (UsageRange::Today, Some(true)) if u.day.is_none() && self.current_filter(cx).project.is_none() => {
                if (snap.totals.cost.total() - self.today.cost.total()).abs() < 0.005 { "yes" } else { "no" }
            }
            _ => "-",
        };
        let filter = self.current_filter(cx);
        let delegate = self.table.read(cx).delegate();
        let focus = if self.table.read(cx).focus_handle(cx).is_focused(window) {
            "table"
        } else if self.search.read(cx).focus_handle(cx).is_focused(window) {
            "search"
        } else if u.focus.is_focused(window) {
            "usage"
        } else {
            "-"
        };
        let usage_sessions = snap.cards.iter().find(|c| c.label == "Sessions").map_or(String::from("-"), |c| c.value.replace(',', ""));
        format!(
            "view={} range={} metric={metric} chart={}/{} hover={} empty={empty} calls={} cost={} project={} width={:.0} {} today-matches={today_matches} \
             active={} updated={} rows={} selected={} detail={} focus={focus} usage-sessions={usage_sessions} toolbar={}",
            if self.view == MainView::Usage { "usage" } else { "sessions" },
            snap.range_label,
            if chart.hourly { "hourly" } else { "daily" },
            chart.slots.len(),
            u.hover.map_or("-".to_string(), |ix| ix.to_string()),
            snap.totals.responses,
            fmt_money(snap.totals.cost.total()),
            self.current_filter(cx).project.as_deref().unwrap_or("-"),
            window.viewport_size().width.as_f32(),
            self.today_label,
            filter.active.map_or("-".to_string(), |a| format!("{}..{}", a.first, a.last)),
            if filter.since_ms.is_none() && filter.until_ms.is_none() { "any" } else { "limited" },
            delegate.ix.len(),
            self.selected.as_ref().map_or("-", |row| row.short_id.as_ref()),
            if self.detail_visible { "yes" } else { "no" },
            match (self.view, self.session_toolbar.get()) {
                (MainView::Usage, _) => "-".to_string(),
                (MainView::Sessions, (rows, inside)) => format!("{rows}/{}", if inside { "inside" } else { "overflow" }),
            },
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use core::prelude::v1::test;

    use agent_session_core::model::{HourlyUsage, LedgerSession, SessionUsage};
    use agent_session_core::pricing::{price, price_tokens};
    use chrono::{FixedOffset, Timelike};

    use super::*;

    pub(crate) fn tz() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    pub(crate) fn ms(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis()
    }

    pub(crate) fn day(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    /// 15:53 on Monday 2026-11-16, +8.
    pub(crate) fn now() -> i64 {
        ms("2026-11-16T15:53:00+08:00")
    }

    /// Tokens of one call in these tests.
    const CALL: Tokens = Tokens { input: 1_000, cache_write_5m: 0, cache_write_1h: 2_000, cache_read: 47_000, output: 500, web_search: 0 };

    pub(crate) fn times(n: u64) -> Tokens {
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
    pub(crate) fn cost(model: &str, n: u64) -> f64 {
        price_tokens(&price(model).unwrap(), &times(n), false, false).total()
    }

    /// `n` calls of `model` by `session` (of project `/p/<project>`) in the hour starting at `at`.
    pub(crate) struct Calls<'a> {
        session: &'a str,
        project: &'a str,
        at: &'a str,
        model: &'a str,
        n: u64,
    }

    pub(crate) fn ledger(calls: &[Calls]) -> UsageData {
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

    pub(crate) fn query(range: UsageRange) -> UsageQuery {
        UsageQuery { range, custom: None, day: None, metric: Metric::Cost, filter: UsageFilter::default() }
    }

    pub(crate) fn build(data: &UsageData, q: &UsageQuery) -> UsageSnapshot {
        UsageSnapshot::build(data, Some(true), q, now(), day("2026-11-16"), &tz())
    }

    pub(crate) const OPUS: &str = "claude-opus-5-5";
    pub(crate) const SONNET: &str = "claude-sonnet-5-5";

    /// Today: 3 Opus calls in s1 (alpha) at 09:00, 1 Sonnet call in s2 (beta) at 15:00.
    /// Yesterday: 2 Opus calls at 10:00 and 2 at 17:00 (after the hour of now).
    pub(crate) fn two_days() -> UsageData {
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

    #[test]
    fn a_range_outside_this_year_says_its_year() {
        let data = two_days();
        let custom = |first: &str, last: &str| {
            let mut q = query(UsageRange::Custom);
            q.custom = Some((day(first), day(last)));
            build(&data, &q)
        };
        assert_eq!(custom("2026-09-01", "2026-09-30").range_label, "Sep 1 – Sep 30", "this year goes without");
        assert_eq!(custom("2000-01-03", "2000-01-09").range_label, "Jan 3 – Jan 9, 2000");
        assert_eq!(custom("2025-12-28", "2026-01-03").range_label, "Dec 28, 2025 – Jan 3, 2026", "across the new year, both ends");
        assert_eq!(custom("2025-07-04", "2025-07-04").range_label, "Fri Jul 4, 2025");
        assert_eq!(custom("2025-07-04", "2025-07-04").chart.subtitle, "Fri Jul 4, 2025", "the chart's subtitle too");
        let mut q = query(UsageRange::Day);
        q.day = Some(day("2025-12-31"));
        assert_eq!(build(&data, &q).range_label, "Wed Dec 31, 2025", "a day of last year clicked on the chart");
        q.day = Some(day("2026-11-13"));
        assert_eq!(build(&data, &q).range_label, "Fri Nov 13");
    }

    /// What an axis label says, back as a number: `$1.5k`, `$0.005`, `2.5M`, `99.25%`.
    fn read_axis(label: &str) -> f64 {
        let s = label.trim_start_matches('$');
        let (number, scale) = if let Some(n) = s.strip_suffix('%') {
            (n, 0.01)
        } else if let Some(n) = s.strip_suffix('k') {
            (n, 1e3)
        } else if let Some(n) = s.strip_suffix('M') {
            (n, 1e6)
        } else if let Some(n) = s.strip_suffix('B') {
            (n, 1e9)
        } else {
            (s, 1.0)
        };
        number.parse::<f64>().unwrap_or_else(|_| panic!("not a number: {label}")) * scale
    }

    /// No two lines of the value axis read alike, and each says its own value.
    fn assert_axis(chart: &ChartData, what: &str) {
        let labels: Vec<&str> = chart.ticks.iter().map(|(_, l)| l.as_ref()).collect();
        let distinct: std::collections::BTreeSet<&str> = labels.iter().copied().collect();
        assert_eq!(distinct.len(), labels.len(), "{what}: {labels:?}");
        let step = chart.ticks[1].0 - chart.ticks[0].0;
        for (value, label) in &chart.ticks {
            assert!((read_axis(label) - value).abs() <= step * 1e-3, "{what}: {label} for {value} ({labels:?})");
        }
    }

    #[test]
    fn the_value_axis_says_each_line_s_value_and_no_two_alike() {
        // n calls in one hour of today: from one call of Haiku (a tenth of a cent) to millions of Opus
        let mut n = 1u64;
        while n < 50_000_000 {
            for model in ["claude-haiku-5-5", OPUS] {
                let data = ledger(&[Calls { session: "s", project: "p", at: "2026-11-16T09:00:00+08:00", model, n }]);
                for metric in [Metric::Cost, Metric::In, Metric::Out] {
                    let mut q = query(UsageRange::Today);
                    q.metric = metric;
                    assert_axis(&build(&data, &q).chart, &format!("{n} calls of {model} by {metric:?}"));
                }
            }
            n = n * 3 / 2 + 1;
        }
        // a range without calls: the bare axes
        for metric in [Metric::Cost, Metric::In, Metric::Out, Metric::HitRate] {
            let mut q = query(UsageRange::Custom);
            q.custom = Some((day("2000-01-03"), day("2000-01-09")));
            q.metric = metric;
            assert_axis(&build(&two_days(), &q).chart, &format!("nothing by {metric:?}"));
        }
        // the hit rate, from a little under its lowest point to 100%
        for read in [0, 300, 650, 900, 940, 975, 990, 995, 998, 999, 1_000] {
            let mut data = UsageData::default();
            data.replace_all(vec![SessionUsage {
                session: LedgerSession { id: "s".into(), project_key: "-p-p".into(), ..Default::default() },
                hours: vec![HourlyUsage {
                    hour: ms("2026-11-16T09:00:00+08:00") / 3_600_000,
                    model: OPUS.into(),
                    responses: 1,
                    tokens: Tokens { input: 1_000 - read, cache_read: read, output: 10, ..Default::default() },
                    ..Default::default()
                }],
            }]);
            let mut q = query(UsageRange::Today);
            q.metric = Metric::HitRate;
            assert_axis(&build(&data, &q).chart, &format!("a hit rate of {read}‰"));
        }
    }

    /// `n` calls a clock hour for each hour of `days` in `tz`: the hour plus 1, and 100 more the
    /// second time a clock hour comes round (the night the clocks go back).
    fn every_hour<T: TimeZone>(days: &[NaiveDate], tz: &T) -> UsageData {
        let mut at = Vec::new();
        for d in days {
            let mut seen = std::collections::BTreeMap::new();
            for span in hour_slots(*d, tz) {
                let clock = local(span.from_ms, tz).hour() as u64;
                let k = seen.entry(clock).or_insert(0);
                at.push((chrono::DateTime::from_timestamp_millis(span.from_ms).unwrap().to_rfc3339(), clock + 1 + 100 * *k));
                *k += 1;
            }
        }
        let calls: Vec<Calls> = at.iter().map(|(at, n)| Calls { session: "s", project: "p", at, model: OPUS, n: *n }).collect();
        ledger(&calls)
    }

    /// The hours of `d` charted in `tz`: each one's label and the calls in its dashed box.
    fn hours_and_boxes<T: TimeZone>(data: &UsageData, d: NaiveDate, tz: &T) -> Vec<(String, Option<u64>)> {
        let mut q = query(UsageRange::Custom);
        q.custom = Some((d, d));
        let snap = UsageSnapshot::build(data, Some(true), &q, ms("2026-12-01T12:00:00Z"), day("2026-12-01"), tz);
        snap.chart.slots.iter().map(|s| (s.label.to_string(), s.ghost.map(|g| g.responses))).collect()
    }

    fn hour(label: &str, boxed: Option<u64>) -> (String, Option<u64>) {
        (label.to_string(), boxed)
    }

    /// The day's k-th 01:00 stands in front of the k-th 01:00 of the day before; an hour the
    /// day before did not have gets no box.
    #[test]
    fn the_box_behind_an_hour_is_the_same_clock_hour_the_day_before_across_dst() {
        let tz = &chrono_tz::America::New_York;
        // spring: 02:00 on Sunday Mar 8 never comes; it is in the usage day of Mar 7 (04:00 to 04:00)
        let data = every_hour(&[day("2026-03-06"), day("2026-03-07"), day("2026-03-08")], tz);
        let short = hours_and_boxes(&data, day("2026-03-07"), tz);
        assert_eq!(short.len(), 23);
        assert_eq!(short[20..], [hour("00", Some(1)), hour("01", Some(2)), hour("03", Some(4))], "03:00 in front of 03:00, not of 02:00");
        let after = hours_and_boxes(&data, day("2026-03-08"), tz);
        assert_eq!(after.len(), 24);
        assert_eq!(after[21..], [hour("01", Some(2)), hour("02", None), hour("03", Some(4))], "the day before had no 02:00");
        // autumn: 01:00 comes twice on Sunday Nov 1, in the usage day of Oct 31
        let data = every_hour(&[day("2026-10-30"), day("2026-10-31"), day("2026-11-01")], tz);
        let long = hours_and_boxes(&data, day("2026-10-31"), tz);
        assert_eq!(long.len(), 25);
        assert_eq!(
            long[20..],
            [hour("00", Some(1)), hour("01", Some(2)), hour("01", None), hour("02", Some(3)), hour("03", Some(4))],
            "the second 01:00 has none the day before"
        );
        let after = hours_and_boxes(&data, day("2026-11-01"), tz);
        assert_eq!(after[20..], [hour("00", Some(1)), hour("01", Some(2)), hour("02", Some(3)), hour("03", Some(4))], "01:00 in front of the first 01:00");
        // every other hour faces its own clock hour
        for (label, boxed) in short.iter().chain(&long).filter(|(l, _)| l.as_str() > "03") {
            assert_eq!(*boxed, Some(label.parse::<u64>().unwrap() + 1), "{label}");
        }
    }
}
