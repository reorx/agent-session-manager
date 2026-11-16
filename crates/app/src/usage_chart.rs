//! The Usage view's chart (`UsageSnapshot::chart`): stacked bars by model, a day by the hour or a
//! bar a day. Behind each hour a dashed box is the same hour of the day before; over the days a
//! dashed line is the 7-day moving average; the hit rate is a line; on today's hours a line marks
//! now.
//!
//! The geometry is the pure `Layout`: the plot area inside the chart's bounds, where each bar is,
//! and `slot_at`, which turns a horizontal position into a bar. Drawing, the tooltip and a click
//! on a day (`AppRoot::click_chart`, on the div around the chart, since `Plot` has no click) all
//! go through it, so they cannot disagree on which bar is where.

use std::cell::Cell;
use std::rc::Rc;

use agent_session_core::format::fmt_tokens;
use agent_session_core::usage::Metric;
use chrono::{Local, TimeZone};
use gpui_kit::component::plot::label::{PlotLabel, Text};
use gpui_kit::component::plot::shape::{Bar, Line};
use gpui_kit::component::plot::tooltip::{CrossLine, Dot, Tooltip, TooltipState};
use gpui_kit::component::plot::{AxisLabelSide, AxisText, Grid, IntoPlot, Plot, PlotAxis, StrokeStyle};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::AppRoot;
use crate::resident::thousands;
use crate::style::{self, FluentBuilder};
use crate::usage_view::{ChartData, MODEL_SLOT_NAMES, fmt_day, fmt_metric, fmt_pct, metric_value};

/// Room left of the plot area for the value labels.
const LEFT: f32 = 56.;
/// Room right of it: a little by the hour, the moving average's label by the day.
const RIGHT_HOURS: f32 = 20.;
const RIGHT_DAYS: f32 = 86.;
/// Room above it for the time of now, below it for the slot labels.
const TOP: f32 = 22.;
const BOTTOM: f32 = 28.;
/// A bar takes this share of its band, within these widths.
const BAR_SHARE: f32 = 0.64;
const MIN_BAR: f32 = 3.;
const MAX_BAR_HOURS: f32 = 22.;
const MAX_BAR_DAYS: f32 = 26.;
/// The band a day's label (`11-16`) needs: narrower bands label every second, third, … day.
const DAY_LABEL_W: f32 = 44.;
/// The white line between two stacked pieces, and the least a piece shows of itself.
const SEGMENT_GAP: f32 = 1.5;
const MIN_SEGMENT: f32 = 0.6;
/// The text of the axes and the marks.
const TEXT_SIZE: f32 = 10.5;
/// Room between the mouse (or a pinned bar) and the side of the tooltip's box.
const TOOLTIP_GAP: f32 = 12.;
/// The tooltip's box in tests (`VisualTestContext::debug_bounds`).
pub const TOOLTIP_SELECTOR: &str = "usage-chart-tooltip";

/// One stacked piece of a bar: a model color slot and where it starts and ends (px from the
/// chart's top; `top` < `bottom`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub slot: usize,
    pub top: f32,
    pub bottom: f32,
    /// The top piece of the bar: its upper corners are rounded.
    pub cap: bool,
}

impl Segment {
    /// Where the piece is drawn from: a hair under its top, so the one above stands apart.
    fn drawn_top(&self) -> f32 {
        if self.cap { self.top } else { (self.top + SEGMENT_GAP).min(self.bottom - MIN_SEGMENT) }
    }
}

/// Where things are in a chart of `slots` bars `width` × `height` px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The plot area, px from the chart's top-left corner: the value labels are left of it, the
    /// slot labels under it, the moving average's label right of it.
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub slots: usize,
    /// The width of a slot: its bar and the room around it.
    pub band: f32,
    /// The width of a bar.
    pub bar: f32,
    hourly: bool,
}

impl Layout {
    pub fn new(width: f32, height: f32, slots: usize, hourly: bool) -> Self {
        let right = if hourly { RIGHT_HOURS } else { RIGHT_DAYS };
        let plot_w = (width - LEFT - right).max(1.);
        let band = plot_w / slots.max(1) as f32;
        let widest = if hourly { MAX_BAR_HOURS } else { MAX_BAR_DAYS };
        Layout {
            left: LEFT,
            top: TOP,
            width: plot_w,
            height: (height - TOP - BOTTOM).max(1.),
            slots,
            band,
            // at least 3 px, but never wider than its band: a long Custom range has bands
            // under 3 px, and a bar reaching into the next day's band would be clicked as that day
            bar: (band * BAR_SHARE).min(widest).max(MIN_BAR.min(band)),
            hourly,
        }
    }

    /// The layout of `chart` painted in `bounds`.
    pub fn of(bounds: Bounds<Pixels>, chart: &ChartData) -> Self {
        Layout::new(bounds.size.width.as_f32(), bounds.size.height.as_f32(), chart.slots.len(), chart.hourly)
    }

    /// The bottom of the plot area: the value axis' `y_min`.
    pub fn bottom(&self) -> f32 {
        self.top + self.height
    }

    /// The left edge of slot `ix`'s band.
    pub fn band_left(&self, ix: usize) -> f32 {
        self.left + self.band * ix as f32
    }

    /// The left edge of slot `ix`'s bar, centered in its band.
    pub fn bar_left(&self, ix: usize) -> f32 {
        self.center(ix) - self.bar / 2.
    }

    pub fn center(&self, ix: usize) -> f32 {
        self.band_left(ix) + self.band / 2.
    }

    /// The height of `value` on an axis from `y_min` (the bottom) to `y_max` (the top).
    pub fn y(&self, value: f64, y_min: f64, y_max: f64) -> f32 {
        if y_max <= y_min {
            return self.bottom();
        }
        let up = ((value - y_min) / (y_max - y_min)) as f32;
        self.top + self.height * (1. - up)
    }

    /// The slot whose band is under `x` (px from the chart's left edge); `None` over the value
    /// labels and right of the last bar.
    pub fn slot_at(&self, x: f32) -> Option<usize> {
        if self.slots == 0 || x < self.left || x >= self.left + self.width {
            return None;
        }
        let mut ix = (((x - self.left) / self.band).floor() as usize).min(self.slots - 1);
        // the same edges as `band_left`, whatever the rounding of the division
        if ix + 1 < self.slots && x >= self.band_left(ix + 1) {
            ix += 1;
        } else if ix > 0 && x < self.band_left(ix) {
            ix -= 1;
        }
        Some(ix)
    }

    /// Whether slot `ix` gets a label under it: every third hour from 04; days are counted back
    /// from the last one (today is always labeled), as many as fit.
    pub fn labeled(&self, ix: usize) -> bool {
        if ix >= self.slots {
            return false;
        }
        if self.hourly {
            return ix % 3 == 0;
        }
        let every = ((DAY_LABEL_W / self.band).ceil() as usize).max(1);
        (self.slots - 1 - ix) % every == 0
    }
}

/// A bar's pieces from the bottom up, each starting where the one below ends; models without a
/// value are left out. `values` by color slot, `y` maps a value to its height.
pub fn stack(values: [f64; 5], y: impl Fn(f64) -> f32) -> Vec<Segment> {
    let mut sum = 0.0;
    let mut segments = Vec::new();
    for (slot, value) in values.into_iter().enumerate().filter(|(_, v)| *v > 0.0) {
        let bottom = y(sum);
        sum += value;
        segments.push(Segment { slot, top: y(sum), bottom, cap: false });
    }
    if let Some(top) = segments.last_mut() {
        top.cap = true;
    }
    segments
}

/// What the tooltip of a bar says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tip {
    /// `Mon Nov 16 · 09:00–10:00`, `Fri Nov 13`, `Mon Nov 16 · so far`.
    pub title: SharedString,
    /// The bar's models, bottom first: color slot, name, value.
    pub models: Vec<(usize, SharedString, SharedString)>,
    /// `Total`, `In · hit · Out`, `Calls`, `7-day average` / `Day before`: label, value.
    pub lines: Vec<(SharedString, SharedString)>,
    /// `Not yet`, `No calls`, or `Click to see its hours`.
    pub note: Option<SharedString>,
}

/// The tooltip of bar `ix`. Hours are written in `tz`.
pub fn tip<T: TimeZone>(chart: &ChartData, ix: usize, tz: &T) -> Option<Tip> {
    let slot = chart.slots.get(ix)?;
    let clock = |ms: i64| tz.timestamp_millis_opt(ms).earliest().map(|t| t.naive_local().format("%H:%M").to_string()).unwrap_or_default();
    let title = if chart.hourly {
        format!("{} · {}–{}", fmt_day(slot.day), clock(slot.span.from_ms), clock(slot.span.until_ms))
    } else if slot.partial {
        format!("{} · so far", fmt_day(slot.day))
    } else {
        fmt_day(slot.day)
    };
    if slot.future {
        return Some(Tip { title: title.into(), note: Some("Not yet".into()), ..Default::default() });
    }
    // the hit rate is a line, not a stack: the tooltip still says what each model cost
    let unit = if chart.metric == Metric::HitRate { Metric::Cost } else { chart.metric };
    let value = |t| fmt_metric(unit, metric_value(t, unit).unwrap_or(0.0)).into();
    let models = (0..MODEL_SLOT_NAMES.len()).filter(|&m| slot.by_model[m].responses > 0).map(|m| (m, MODEL_SLOT_NAMES[m].into(), value(&slot.by_model[m]))).collect();
    let mut lines: Vec<(SharedString, SharedString)> = Vec::new();
    let t = &slot.totals;
    if t.responses > 0 {
        lines.push(("Total".into(), value(t)));
        let input = t.tokens.input_total();
        if input > 0 {
            let hit = fmt_pct(t.tokens.cache_read as f64 / input as f64);
            lines.push(("In · hit · Out".into(), format!("{} · {hit} · {}", fmt_tokens(input), fmt_tokens(t.tokens.output)).into()));
        }
        lines.push(("Calls".into(), thousands(t.responses).into()));
    }
    if let Some(average) = slot.average {
        lines.push(("7-day average".into(), fmt_metric(chart.metric, average).into()));
    }
    if let Some(before) = &slot.ghost {
        lines.push(("Day before".into(), value(before)));
    }
    let note = if t.responses == 0 {
        Some("No calls")
    } else if !chart.hourly {
        Some("Click to see its hours")
    } else {
        None
    };
    Some(Tip { title: title.into(), models, lines, note: note.map(Into::into) })
}

/// The chart in the Usage view's chart area. Where it was last painted goes into `bounds` for
/// `AppRoot::click_chart`; `pinned` shows that bar's tooltip without the mouse (autoplay's hover
/// step: the mouse cannot be moved there without taking the user's).
#[derive(IntoPlot)]
pub struct UsageChart {
    chart: ChartData,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    pinned: Option<usize>,
}

impl UsageChart {
    pub fn new(chart: ChartData, bounds: Rc<Cell<Bounds<Pixels>>>, pinned: Option<usize>) -> Self {
        UsageChart { chart, bounds, pinned }
    }

    /// The tooltip of bar `ix`, its column lit, its box beside `x` (the mouse, or the pinned
    /// bar's center) with its top at the plot area's: a box opening upwards from a mouse low in
    /// the chart climbed over the title row and the legend.
    fn tooltip_box(&self, ix: usize, x: Pixels, bounds: Bounds<Pixels>, cx: &App) -> Option<AnyElement> {
        let tip = tip(&self.chart, ix, &Local)?;
        let l = Layout::of(bounds, &self.chart);
        // `Tooltip` puts the box `gap` below a point in the upper half of the plot
        let cursor = point(x, px(l.top - TOOLTIP_GAP));
        let column = CrossLine::new(point(px(l.center(ix)), px(l.top))).band(px(l.band)).span(l.top - 8., l.height + 8.);
        let dot = (self.chart.metric == Metric::HitRate)
            .then(|| self.chart.slots[ix].value(Metric::HitRate))
            .flatten()
            .map(|v| Dot::new(point(px(l.center(ix)), px(l.y(v, self.chart.y_min, self.chart.y_max)))).size(px(9.)).stroke(style::model_color(0)).fill(style::model_color(0)));
        Some(Tooltip::new(cursor, bounds.size).gap(px(TOOLTIP_GAP)).cross_line(column).dots(dot).child(tip_content(tip, cx)).into_any_element())
    }
}

/// The tooltip's box: the title, a line a model, then the totals and a note.
fn tip_content(tip: Tip, cx: &App) -> Div {
    let line = |label: SharedString, value: SharedString, bold: bool| {
        h_flex()
            .justify_between()
            .gap(px(16.))
            .child(div().text_color(style::text_secondary()).child(label))
            .child(div().font_family(style::mono(cx)).text_size(px(11.5)).when(bold, |el| el.font_weight(FontWeight::SEMIBOLD)).child(value))
    };
    let models = tip.models.into_iter().map(|(slot, name, value)| {
        h_flex()
            .justify_between()
            .gap(px(16.))
            .child(h_flex().gap(px(6.)).items_center().child(div().flex_none().size(px(8.)).rounded(px(2.)).bg(style::model_color(slot))).child(name))
            .child(div().font_family(style::mono(cx)).text_size(px(11.5)).child(value))
    });
    let has_models = models.len() > 0;
    let mut lines = tip.lines.into_iter().enumerate().map(|(ix, (label, value))| line(label.clone(), value, ix == 0 && label == "Total")).peekable();
    let has_lines = lines.peek().is_some();
    v_flex()
        .debug_selector(|| TOOLTIP_SELECTOR.into())
        .min_w(px(190.))
        .gap(px(3.))
        .font_family(style::sans(cx))
        .text_size(px(12.))
        .text_color(style::text())
        .child(div().font_weight(FontWeight::SEMIBOLD).mb(px(2.)).child(tip.title))
        .children(models)
        .when(has_models && has_lines, |el| el.child(div().h(px(1.)).my(px(2.)).bg(style::hairline())))
        .children(lines)
        .when_some(tip.note, |el, note| el.child(div().mt(px(2.)).text_size(px(11.5)).text_color(style::muted()).child(note)))
}

impl Plot for UsageChart {
    /// The pinned tooltip, unless the mouse is over the chart and brings its own.
    fn prepaint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) -> Vec<AnyElement> {
        let Some(ix) = self.pinned.filter(|ix| *ix < self.chart.slots.len()) else { return Vec::new() };
        if bounds.contains(&window.mouse_position()) {
            return Vec::new();
        }
        let l = Layout::of(bounds, &self.chart);
        let Some(mut tooltip) = self.tooltip_box(ix, px(l.center(ix)), bounds, cx) else { return Vec::new() };
        tooltip.prepaint_as_root(bounds.origin, bounds.size.into(), window, cx);
        vec![tooltip]
    }

    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        self.bounds.set(bounds);
        let chart = &self.chart;
        if chart.slots.is_empty() {
            return;
        }
        let l = Layout::of(bounds, chart);
        let (y_min, y_max, metric) = (chart.y_min, chart.y_max, chart.metric);
        let y = move |v: f64| l.y(v, y_min, y_max);
        let at = |x: f32, y: f32| point(bounds.origin.x + px(x), bounds.origin.y + px(y));

        // grid lines across the plot area, values left of it, the axis and the slots under it
        let area = Bounds::new(at(l.left, 0.), size(px(l.width), bounds.size.height));
        Grid::new().y(chart.ticks.iter().skip(1).map(|(v, _)| px(y(*v))).collect::<Vec<_>>()).stroke(style::row_border()).paint(&area, window);
        let slot_labels = chart.slots.iter().enumerate().filter(|(ix, _)| l.labeled(*ix)).map(|(ix, s)| {
            let color = if s.partial {
                style::text()
            } else if s.weekend {
                style::chart_weekend()
            } else {
                style::muted()
            };
            AxisText::new(s.label.clone(), px(l.center(ix) - l.left), color).font_size(px(TEXT_SIZE)).align(TextAlign::Center)
        });
        let value_labels = chart.ticks.iter().map(|(v, label)| AxisText::new(label.clone(), px(y(*v)), style::muted()).font_size(px(TEXT_SIZE)).align(TextAlign::Right));
        PlotAxis::new()
            .x(px(l.bottom()))
            .x_label(slot_labels)
            .y(px(-6.))
            .y_axis(false)
            .y_label_side(AxisLabelSide::Start)
            .y_label(value_labels)
            .stroke(style::chart_axis())
            .paint(&area, window, cx);

        let mut marks = Vec::new();
        if metric == Metric::HitRate {
            let points: Vec<(f32, f32)> = chart.slots.iter().enumerate().filter_map(|(ix, s)| s.value(metric).map(|v| (l.center(ix), y(v)))).collect();
            Line::new()
                .data(points)
                .x(|p| Some(p.0))
                .y(|p| Some(p.1))
                .stroke(style::model_color(0))
                .stroke_width(px(2.))
                .stroke_style(StrokeStyle::Linear)
                .dot()
                .dot_size(px(6.))
                .dot_fill_color(style::white())
                .dot_stroke_color(style::model_color(0))
                .paint(&bounds, window);
        } else {
            // the same hour of the day before, behind the bar
            for (ix, s) in chart.slots.iter().enumerate() {
                let before = s.ghost.as_ref().and_then(|g| metric_value(g, metric)).unwrap_or(0.0);
                if before > 0.0 {
                    let x = l.bar_left(ix) - 1.5;
                    let frame = Bounds::from_corners(at(x, y(before)), at(x + l.bar + 3., l.bottom()));
                    window.paint_quad(outline(frame, style::chart_ghost(), BorderStyle::Dashed).corner_radii(px(3.)));
                }
            }
            let (caps, rest): (Vec<(usize, Segment)>, Vec<(usize, Segment)>) =
                chart.slots.iter().enumerate().flat_map(|(ix, s)| stack(s.stack(metric), y).into_iter().map(move |seg| (ix, seg))).partition(|(_, seg)| seg.cap);
            for (pieces, round) in [(rest, 0.), (caps, 3.)] {
                Bar::new()
                    .data(pieces)
                    .band_width(l.bar)
                    .cross(move |(ix, _)| Some(l.bar_left(*ix)))
                    .base(|(_, seg)| seg.bottom)
                    .value(|(_, seg)| Some(seg.drawn_top()))
                    .fill(|(_, seg), _, _| style::model_color(seg.slot))
                    .corner_radii(Corners { top_left: px(round), top_right: px(round), bottom_right: px(0.), bottom_left: px(0.) })
                    .paint(&bounds, window, cx);
            }
            if !chart.hourly && chart.slots.iter().any(|s| s.average.is_some_and(|a| a > 0.0)) {
                // the moving average, and its last value right of the plot area
                let points: Vec<Point<Pixels>> = chart.slots.iter().enumerate().filter_map(|(ix, s)| s.average.map(|a| at(l.center(ix), y(a)))).collect();
                if let Some((first, others)) = points.split_first() {
                    let mut path = PathBuilder::stroke(px(1.5)).dash_array(&[px(4.), px(3.)]);
                    path.move_to(*first);
                    for p in others {
                        path.line_to(*p);
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, style::chart_average());
                    }
                }
                if let Some(last) = chart.slots.last().and_then(|s| s.average) {
                    let x = l.left + l.width + 8.;
                    marks.push(Text::new("7-day avg", point(px(x), px(y(last) - 14.)), style::muted()).font_size(px(TEXT_SIZE)));
                    marks.push(Text::new(fmt_metric(metric, last), point(px(x), px(y(last) + 1.)), style::text()).font_size(px(TEXT_SIZE)));
                }
                // today's bar is still filling
                if let Some((ix, value)) = chart.slots.iter().enumerate().find(|(_, s)| s.partial).and_then(|(ix, s)| Some((ix, s.value(metric)?))) {
                    if value > 0.0 {
                        marks.push(Text::new("so far", point(px(l.center(ix)), px(y(value) - 15.)), style::muted()).font_size(px(9.5)).align(TextAlign::Center));
                    }
                }
            }
        }
        // now, on today's hours
        if let Some((slots, time)) = &chart.now {
            let x = l.left + l.band * *slots as f32;
            let mut path = PathBuilder::stroke(px(1.)).dash_array(&[px(2.), px(2.)]);
            path.move_to(at(x, l.top - 6.));
            path.line_to(at(x, l.bottom()));
            if let Ok(path) = path.build() {
                window.paint_path(path, style::accent());
            }
            marks.push(Text::new(time.clone(), point(px(x + 4.), px(l.top - 19.)), style::accent()).font_size(px(TEXT_SIZE)));
        }
        PlotLabel::new(marks).paint(&bounds, window, cx);
    }

    fn id(&self) -> Option<ElementId> {
        Some("usage-plot".into())
    }

    fn tooltip_state(&self, position: Point<Pixels>, bounds: Bounds<Pixels>, _: &App) -> Option<TooltipState> {
        let l = Layout::of(bounds, &self.chart);
        let ix = l.slot_at(position.x.as_f32())?;
        Some(TooltipState::new(ix, point(px(l.center(ix)), px(l.top)), Vec::new()))
    }

    fn tooltip(&self, state: &TooltipState, cursor: Point<Pixels>, bounds: Bounds<Pixels>, _: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.tooltip_box(state.index, cursor.x, bounds, cx)
    }
}

impl AppRoot {
    /// A click in the chart area (`Plot` has no click of its own): on a day's bar of the daily
    /// chart, that day by the hour. Anywhere else, or a day without calls: nothing (false).
    pub(crate) fn click_chart(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let u = &self.usage_view;
        let bounds = u.chart_bounds.get();
        let chart = &u.snapshot.chart;
        if chart.hourly || !bounds.contains(&position) {
            return false;
        }
        let Some(ix) = Layout::of(bounds, chart).slot_at((position.x - bounds.origin.x).as_f32()) else { return false };
        let day = chart.slots[ix].day;
        self.drill_into_day(day, cx)
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use agent_session_core::format::fmt_tokens;
    use agent_session_core::usage::{Metric, UsageRange};

    use super::*;
    use crate::usage_view::tests::{OPUS, SONNET, build, cost, query, two_days, tz};
    use crate::usage_view::{fmt_money, fmt_pct};

    #[test]
    fn slot_at_maps_the_cursor_to_the_bar_under_it() {
        let l = Layout::new(800., 236., 24, true);
        assert_eq!(l.slot_at(l.left - 1.), None, "over the value labels");
        assert_eq!(l.slot_at(l.left), Some(0));
        assert_eq!(l.slot_at(l.center(5)), Some(5));
        assert_eq!(l.slot_at(l.band_left(6) - 0.01), Some(5), "the room right of a bar is still its own");
        assert_eq!(l.slot_at(l.band_left(6)), Some(6));
        assert_eq!(l.slot_at(l.band_left(23) + l.band - 0.01), Some(23));
        assert_eq!(l.slot_at(l.left + l.width), None, "right of the last bar");
        for ix in 0..24 {
            assert!(l.bar_left(ix) >= l.band_left(ix) && l.bar_left(ix) + l.bar <= l.band_left(ix) + l.band, "bar {ix} inside its band");
            assert!((l.bar_left(ix) + l.bar / 2. - l.center(ix)).abs() < 1e-3, "bar {ix} centered");
            assert_eq!(l.slot_at(l.bar_left(ix) + 1.), Some(ix));
        }
        assert!((l.band_left(23) + l.band - (l.left + l.width)).abs() < 1e-3, "the bands fill the plot area");

        // the daily chart keeps room on the right for the moving average's label
        let d = Layout::new(800., 236., 30, false);
        assert_eq!(d.left, l.left);
        assert!(d.left + d.width < l.left + l.width);
        // bars are never wider than 26 px nor narrower than 3
        assert_eq!(Layout::new(4000., 236., 7, false).bar, 26.);
        assert_eq!(Layout::new(4000., 236., 24, true).bar, 22.);
        assert_eq!(Layout::new(800., 236., 160, false).bar, 3., "3 px while the band has room for it");
        assert_eq!(Layout::new(800., 236., 0, false).slot_at(300.), None, "no bars, nothing under the cursor");
        assert!(Layout::new(10., 236., 7, false).width > 0., "a sliver of a window still has a plot area");
    }

    /// A long Custom range: bands under 3 px. A bar never reaches into the next day's band, so
    /// what is drawn, its tooltip and its click are the same day.
    #[test]
    fn a_bar_stays_inside_its_band_however_many_days_there_are() {
        for (width, slots) in [(800., 365), (800., 730), (300., 120), (1400., 1000)] {
            let l = Layout::new(width, 236., slots, false);
            assert!(l.band < 3., "{slots} days in {width} px: narrower than the widest least bar");
            assert!(l.bar > 0. && l.bar <= l.band, "{slots} days: bar {} in band {}", l.bar, l.band);
            for ix in 0..slots {
                let (left, right) = (l.bar_left(ix), l.bar_left(ix) + l.bar);
                assert!(left >= l.band_left(ix) - 1e-3 && right <= l.band_left(ix) + l.band + 1e-3, "{slots} days: bar {ix} inside its band");
                let inside = (l.bar * 0.05).min(0.01);
                assert_eq!(l.slot_at(left + inside), Some(ix), "{slots} days: the left edge of bar {ix} is its own");
                assert_eq!(l.slot_at(right - inside), Some(ix), "{slots} days: the right edge of bar {ix} is its own");
            }
        }
    }

    #[test]
    fn values_map_onto_the_plot_area_from_the_bottom() {
        let l = Layout::new(800., 236., 24, true);
        assert!(l.top > 0. && l.bottom() < 236., "room above for the time of now, below for the slot labels");
        assert_eq!(l.y(0., 0., 100.), l.bottom());
        assert_eq!(l.y(100., 0., 100.), l.top);
        assert_eq!(l.y(50., 0., 100.), l.top + l.height / 2.);
        assert_eq!(l.y(0.9, 0.9, 1.0), l.bottom(), "the hit rate's axis starts above 0");
    }

    #[test]
    fn stacked_segments_start_where_the_one_below_ends() {
        let l = Layout::new(800., 236., 24, true);
        let y = |v: f64| l.y(v, 0.0, 10.0);
        let segments = stack([3.0, 0.0, 2.0, 0.0, 1.0], y);
        assert_eq!(segments.iter().map(|s| s.slot).collect::<Vec<_>>(), [0, 2, 4], "models without a value are left out");
        assert_eq!(segments[0].bottom, l.bottom(), "the first one stands on the axis");
        assert_eq!(segments[1].bottom, segments[0].top);
        assert_eq!(segments[2].bottom, segments[1].top);
        assert_eq!(segments[0].top, y(3.0));
        assert_eq!(segments[2].top, y(6.0), "the top is the bar's total");
        assert_eq!(segments.iter().map(|s| s.cap).collect::<Vec<_>>(), [false, false, true], "only the top one is rounded");
        assert!(stack([0.0; 5], y).is_empty());
        assert_eq!(stack([0.0, 4.0, 0.0, 0.0, 0.0], y), [Segment { slot: 1, top: y(4.0), bottom: l.bottom(), cap: true }]);
    }

    #[test]
    fn hours_are_labeled_every_third_and_days_count_back_from_today() {
        let l = Layout::new(800., 236., 24, true);
        assert_eq!((0..24).filter(|&ix| l.labeled(ix)).collect::<Vec<_>>(), [0, 3, 6, 9, 12, 15, 18, 21], "04, 07, 10, …");
        let week = Layout::new(800., 236., 7, false);
        assert!((0..7).all(|ix| week.labeled(ix)), "seven days all fit");
        // thirty days in a narrow chart: as many as fit, counted back from the last
        let month = Layout::new(600., 236., 30, false);
        assert!(month.band < 44.);
        assert!(month.labeled(29) && !month.labeled(28));
        let every = (29..30).chain((0..29).rev()).filter(|&ix| month.labeled(ix)).count();
        assert!(every < 30 && every > 5);
    }

    #[test]
    fn the_tooltip_of_an_hour_lists_its_models_its_tokens_and_the_day_before() {
        let today = build(&two_days(), &query(UsageRange::Today)).chart;
        // 09:00: 3 Opus calls; nothing at 09:00 the day before
        let nine = tip(&today, 5, &tz()).unwrap();
        assert_eq!(nine.title, "Mon Nov 16 · 09:00–10:00");
        assert_eq!(nine.models, [(0, "Opus 5.5".into(), fmt_money(cost(OPUS, 3)).into())]);
        let lines: Vec<(&str, String)> = nine.lines.iter().map(|(l, v)| (l.as_ref(), v.to_string())).collect();
        assert_eq!(
            lines,
            [
                ("Total", fmt_money(cost(OPUS, 3))),
                ("In · hit · Out", format!("{} · {} · {}", fmt_tokens(150_000), fmt_pct(0.94), fmt_tokens(1_500))),
                ("Calls", "3".into()),
                ("Day before", "$0.00".into()),
            ]
        );
        assert_eq!(nine.note, None, "hours are not clicked");
        // 10:00: nothing today, 2 calls the day before
        let ten = tip(&today, 6, &tz()).unwrap();
        assert!(ten.models.is_empty());
        assert_eq!(ten.note.as_deref(), Some("No calls"));
        assert_eq!(ten.lines, [("Day before".into(), fmt_money(cost(OPUS, 2)).into())]);
        // 16:00 has not come yet (now is 15:53)
        let later = tip(&today, 12, &tz()).unwrap();
        assert_eq!(later.title, "Mon Nov 16 · 16:00–17:00");
        assert_eq!((later.models.len(), later.lines.len(), later.note.as_deref()), (0, 0, Some("Not yet")));
        assert_eq!(tip(&today, 24, &tz()), None, "no such bar");

        // Out: the models and the total in output tokens
        let mut q = query(UsageRange::Today);
        q.metric = Metric::Out;
        let out = tip(&build(&two_days(), &q).chart, 11, &tz()).unwrap();
        assert_eq!(out.models, [(1, "Sonnet 5.5".into(), fmt_tokens(500).into())]);
        assert_eq!(out.lines[0], ("Total".into(), fmt_tokens(500).into()));
    }

    #[test]
    fn the_legend_lists_the_dashed_marks_only_when_there_are_some() {
        use crate::usage_view::Legend;
        // a week in 2000: bare axes, no average to draw
        let mut q = query(UsageRange::Custom);
        q.custom = Some(("2000-01-03".parse().unwrap(), "2000-01-09".parse().unwrap()));
        let empty = build(&two_days(), &q).chart;
        assert_eq!(empty.slots.len(), 7);
        assert!(empty.legend.is_empty(), "{:?}", empty.legend);
        // yesterday by the hour has nothing the day before it
        let yesterday = build(&two_days(), &query(UsageRange::Yesterday)).chart;
        assert_eq!(yesterday.legend, [Legend::Model(0, "Opus 5.5".into())]);
        // today does: yesterday's hours are its dashed boxes
        assert_eq!(build(&two_days(), &query(UsageRange::Today)).chart.legend.last(), Some(&Legend::Ghost));
        assert_eq!(build(&two_days(), &query(UsageRange::Days7)).chart.legend.last(), Some(&Legend::Average));
    }

    #[test]
    fn the_tooltip_of_a_day_has_the_average_and_says_a_click_opens_it() {
        let week = build(&two_days(), &query(UsageRange::Days7)).chart;
        let today = tip(&week, 6, &tz()).unwrap();
        assert_eq!(today.title, "Mon Nov 16 · so far");
        assert_eq!(today.models.iter().map(|m| m.0).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(today.lines.last().unwrap(), &("7-day average".into(), fmt_money((cost(OPUS, 4) + cost(OPUS, 3) + cost(SONNET, 1)) / 7.0).into()));
        assert_eq!(today.note.as_deref(), Some("Click to see its hours"));
        let empty = tip(&week, 3, &tz()).unwrap();
        assert_eq!(empty.title, "Fri Nov 13");
        assert_eq!(empty.note.as_deref(), Some("No calls"), "an empty day does not open");
        assert_eq!(empty.lines.len(), 1, "only the average");

        // the hit rate is a line: the tooltip lists the cost by model, and the rate itself
        let mut q = query(UsageRange::Days7);
        q.metric = Metric::HitRate;
        let hit = tip(&build(&two_days(), &q).chart, 5, &tz()).unwrap();
        assert_eq!(hit.title, "Sun Nov 15");
        assert_eq!(hit.models, [(0, "Opus 5.5".into(), fmt_money(cost(OPUS, 4)).into())]);
        assert!(hit.lines.iter().all(|(l, _)| l != "7-day average"), "no average of a rate");
    }
}
