//! Detail panel, Cost tab: the selected session's main model with its prices, what each kind
//! of token cost, and the same calls priced as another main model (`cost::compare`). The model
//! picked here also prices the whole table in the status bar while the tab is open.

use std::sync::Arc;
use std::time::Duration;

use agent_session_core::cost::{Comparison, compare};
use agent_session_core::format::{fmt_change, fmt_tokens, fmt_usd};
use agent_session_core::pricing::{CURRENT_MODELS, EARLIER_MODELS, LONG_PROMPT, ModelPrice, display_name, price};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::AppRoot;
use crate::detail::DetailTab;
use crate::model::SessionRow;
use crate::style::{self, FluentBuilder};
use crate::toolbar::{menu_item, trigger};

/// What a session is compared with until another model is picked.
pub const DEFAULT_COMPARE_MODEL: &str = "claude-fable-5-2";

const TOKENS_W: f32 = 64.;
const USD_W: f32 = 86.;
const CHANGE_W: f32 = 56.;

/// `$4.00`, `$0.20`, `$0.125`: per-MTok rates keep a third decimal when they have one.
fn fmt_rate(usd: f64) -> String {
    let text = format!("{usd:.3}");
    format!("${}", text.strip_suffix('0').unwrap_or(&text))
}

/// `Input $4.00 · Output $20.00 · …` per million tokens.
fn rates(p: &ModelPrice) -> String {
    format!(
        "Input {} · Output {} · Cache write {} / {} (1h) · Cache read {} per MTok",
        fmt_rate(p.input),
        fmt_rate(p.output),
        fmt_rate(p.cache_write_5m()),
        fmt_rate(p.cache_write_1h()),
        fmt_rate(p.cache_read),
    )
}

/// `$8 / $40`: the hint beside a model in the Compare with menu.
fn menu_rates(p: &ModelPrice) -> String {
    let short = |usd: f64| if usd.fract() == 0.0 { format!("${usd:.0}") } else { fmt_rate(usd) };
    format!("{} / {}", short(p.input), short(p.output))
}

fn change_color(from: f64, to: f64) -> Hsla {
    if from <= 0.0 || (to - from).abs() / from < 0.005 {
        style::text_secondary()
    } else if to > from {
        style::cost_up()
    } else {
        style::cost_down()
    }
}

fn section_label(text: &'static str) -> Div {
    div().text_size(px(10.)).font_weight(FontWeight::SEMIBOLD).text_color(style::muted()).child(text.to_uppercase())
}

/// One line of the cost table: a label, then right-aligned tokens, both costs and the change.
fn cost_row(label: &'static str, tokens: Option<String>, actual: Option<f64>, predicted: f64, strong: bool, cx: &App) -> Div {
    let cell = |w: f32, text: String| div().w(px(w)).flex_none().text_right().child(text);
    let change = actual.map_or_else(|| "—".to_string(), |a| fmt_change(a, predicted));
    let color = actual.map_or(style::text_secondary(), |a| change_color(a, predicted));
    h_flex()
        .h(px(24.))
        .items_center()
        .border_b_1()
        .border_color(style::hairline())
        .font_family(style::mono(cx))
        .text_size(px(11.))
        .when(strong, |el| el.font_weight(FontWeight::SEMIBOLD))
        .child(div().flex_1().min_w_0().truncate().font_family(style::sans(cx)).text_size(px(12.)).child(label))
        .child(cell(TOKENS_W, tokens.unwrap_or_default()).text_color(style::text_secondary()))
        .child(cell(USD_W, actual.map_or_else(|| "—".into(), fmt_usd)))
        .child(cell(USD_W, fmt_usd(predicted)))
        .child(cell(CHANGE_W, change).text_color(color))
}

fn header_row(source: &str, target: &str, cx: &App) -> Div {
    let cell = |w: f32, text: String| div().w(px(w)).flex_none().text_right().truncate().child(text);
    h_flex()
        .h(px(22.))
        .items_center()
        .border_b_1()
        .border_color(style::border_soft())
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(style::head_fg())
        .child(div().flex_1())
        .child(cell(TOKENS_W, "Tokens".into()))
        .child(cell(USD_W, display_name(source)))
        .child(cell(USD_W, display_name(target)))
        .child(cell(CHANGE_W, "Change".into()))
        .font_family(style::sans(cx))
}

fn note(text: String) -> Div {
    div().text_size(px(11.)).line_height(px(15.)).text_color(style::muted()).child(text)
}

/// What the table's notes say about the calls that were not simply re-priced.
fn notes(c: &Comparison) -> Vec<String> {
    let target = display_name(&c.target);
    let mut notes = Vec::new();
    for other in &c.others {
        let usd = other.cost_usd.map_or_else(|| "no price".to_string(), fmt_usd);
        notes.push(format!("{} keeps its price: {} calls, {usd}, in both totals.", display_name(&other.model), other.responses));
    }
    if c.fast_responses > 0 {
        let fast = price(&c.target).is_some_and(|p| p.fast_mult > 1.0);
        notes.push(match fast {
            true => format!("{} calls ran in fast mode and are priced at {target}'s fast rate too.", c.fast_responses),
            false => format!("{} calls ran in fast mode; {target} has none, so they are priced at its standard rate.", c.fast_responses),
        });
    }
    if c.long_prompt_responses > 0 && price(&c.target).is_some_and(|p| p.long_prompt.is_some()) {
        notes.push(format!("{} calls had prompts over {}, which {target} prices higher.", c.long_prompt_responses, fmt_tokens(LONG_PROMPT)));
    }
    notes.push("Same token counts at another price: a different model would not have run the same conversation.".into());
    notes
}

impl AppRoot {
    pub fn set_compare_model(&mut self, model: SharedString, cx: &mut Context<Self>) {
        self.compare_model = model;
        self.sync_projection(cx);
        self.persist(cx);
        cx.notify();
    }

    /// The status bar prices the table with the compared model while the Cost tab is in view.
    pub(crate) fn sync_projection(&mut self, cx: &mut Context<Self>) {
        let target = (self.detail_visible && self.detail.tab == DetailTab::Cost).then(|| self.compare_model.clone());
        self.table.update(cx, |table, _| table.delegate_mut().set_compare_model(target));
    }

    fn compare_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let current = self.compare_model.clone();
        move |menu, _, _| {
            let mut menu = menu.min_w(px(200.));
            for (ix, model) in CURRENT_MODELS.iter().chain(EARLIER_MODELS).enumerate() {
                if ix == CURRENT_MODELS.len() {
                    menu = menu.separator();
                }
                let Some(p) = price(model) else { continue };
                let this = this.clone();
                let checked = current.as_ref() == *model;
                menu = menu.item(menu_item(display_name(model).into(), menu_rates(&p).into(), true, checked, move |cx| {
                    this.update(cx, |app, cx| app.set_compare_model((*model).into(), cx)).ok();
                }));
            }
            menu
        }
    }

    pub(crate) fn render_cost(&self, row: &Arc<SessionRow>, cx: &mut Context<Self>) -> AnyElement {
        let Some(c) = compare(&row.model_usage, &self.compare_model) else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(style::muted())
                .child("No API usage recorded for this session")
                .into_any_element();
        };
        let source_price = price(&c.source);
        let target_price = price(&c.target);
        let calls = match c.fast_responses {
            0 => format!("{} calls", c.responses),
            fast => format!("{} calls ({fast} fast)", c.responses),
        };
        let a = c.actual;
        let p = c.predicted;
        let t = c.tokens;
        let tokens = |n: u64| Some(fmt_tokens(n));
        let line = |label, n: u64, actual: Option<f64>, predicted: f64| cost_row(label, tokens(n), actual, predicted, false, cx);
        div()
            .id("cost")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(
                v_flex()
                    .px(px(14.))
                    .pt(px(12.))
                    .pb(px(16.))
                    .gap(px(4.))
                    .child(section_label("Model"))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .items_baseline()
                            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(display_name(&c.source)))
                            .child(div().font_family(style::mono(cx)).text_size(px(11.)).text_color(style::text_secondary()).child(format!("{} · {calls}", c.source))),
                    )
                    .child(note(source_price.as_ref().map_or_else(|| format!("No price for {}", c.source), rates)))
                    .child(
                        h_flex()
                            .mt(px(10.))
                            .gap(px(8.))
                            .items_center()
                            .child(section_label("Compare with"))
                            .child(trigger("compare-model", "", display_name(&c.target).into(), cx).dropdown_menu(self.compare_menu(cx))),
                    )
                    .child(note(target_price.as_ref().map(rates).unwrap_or_default()))
                    .child(div().mt(px(12.)).child(section_label("Cost")))
                    .child(header_row(&c.source, &c.target, cx))
                    .child(line("Input", t.input, a.map(|a| a.input), p.input))
                    .when(t.cache_write_5m > 0, |el| el.child(line("Cache write 5m", t.cache_write_5m, a.map(|a| a.cache_write_5m), p.cache_write_5m)))
                    .when(t.cache_write_1h > 0, |el| el.child(line("Cache write 1h", t.cache_write_1h, a.map(|a| a.cache_write_1h), p.cache_write_1h)))
                    .when(t.cache_read > 0, |el| el.child(line("Cache read", t.cache_read, a.map(|a| a.cache_read), p.cache_read)))
                    .child(cost_row("Input total", tokens(t.input_total()), a.map(|a| a.input_total()), p.input_total(), true, cx))
                    .child(cost_row("Output", tokens(t.output), a.map(|a| a.output), p.output, true, cx))
                    .when(t.web_search > 0, |el| el.child(cost_row("Web search", Some(t.web_search.to_string()), a.map(|a| a.web_search), p.web_search, false, cx)))
                    .when(!c.others.is_empty(), |el| el.child(cost_row("Other models", None, a.map(|_| c.others_usd), c.others_usd, false, cx)))
                    .child(cost_row("Total", None, c.actual_total(), c.predicted_total(), true, cx).border_b_0().border_t_1().border_color(style::border_soft()))
                    .child(v_flex().mt(px(10.)).gap(px(4.)).children(notes(&c).into_iter().map(note))),
            )
            .into_any_element()
    }
}

type Step = Box<dyn Fn(&mut AppRoot, &mut Context<AppRoot>) -> bool>;

impl AppRoot {
    /// Scripted tour of the Cost tab for screenshots (`make screenshot-cost`): Opus 5.5 sessions
    /// by context, compared with Fable 5.2 then Sonnet 5.5; a session whose subagents ran on
    /// another model, compared with Haiku 5.5; the same filter sorted by In. Once the start-up
    /// scan is over, every step prints `autoplay-cost: <step>` (or `… skipped`) and holds the
    /// view for a capture.
    pub(crate) fn cost_autoplay(&self, window: &Window, cx: &mut Context<Self>) -> Task<()> {
        let steps: Vec<(&str, Step)> = vec![
            ("opus-by-context", Box::new(|this, cx| {
                this.set_model_filter(Some("claude-opus-5-5".into()), cx);
                this.autoplay_sort_and_select("context", cx);
                this.set_detail_tab(DetailTab::Cost, cx);
                this.set_compare_model(DEFAULT_COMPARE_MODEL.into(), cx);
                true
            })),
            ("as-sonnet", Box::new(|this, cx| {
                this.set_compare_model("claude-sonnet-5-5".into(), cx);
                true
            })),
            ("other-models", Box::new(|this, cx| {
                let delegate = this.table.read(cx).delegate();
                let has_others = |ix: &usize| compare(&delegate.row(*ix).model_usage, DEFAULT_COMPARE_MODEL).is_some_and(|c| !c.others.is_empty());
                let Some(ix) = (0..delegate.ix.len()).find(has_others) else { return false };
                this.table.update(cx, |table, cx| table.scroll_to_row(ix, cx));
                this.select_row(ix, cx);
                this.set_compare_model("claude-haiku-5-5".into(), cx);
                true
            })),
            ("by-input", Box::new(|this, cx| {
                this.autoplay_sort_and_select("input", cx);
                this.set_compare_model(DEFAULT_COMPARE_MODEL.into(), cx);
                true
            })),
        ];
        cx.spawn_in(window, async move |this, cx| {
            for _ in 0..1200 {
                let scanning = this.update(cx, |this, _| matches!(this.index_status, crate::IndexStatus::Loading | crate::IndexStatus::Scanning { .. }));
                if !scanning.unwrap_or(false) {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(100)).await;
            }
            cx.background_executor().timer(Duration::from_millis(500)).await;
            for (name, step) in steps {
                let Ok(ran) = this.update(cx, |this, cx| step(this, cx)) else { return };
                if !ran {
                    println!("autoplay-cost: {name} skipped");
                    continue;
                }
                this.update(cx, |this, cx| {
                    this.status = format!("autoplay: {name}").into();
                    cx.notify();
                })
                .ok();
                cx.background_executor().timer(Duration::from_millis(800)).await;
                println!("autoplay-cost: {name}");
                cx.background_executor().timer(Duration::from_millis(1500)).await;
            }
        })
    }

    fn autoplay_sort_and_select(&mut self, key: &str, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_sort(key, gpui_kit::component::table::ColumnSort::Descending);
            table.refresh(cx);
            table.scroll_to_row(0, cx);
        });
        self.select_row(0, cx);
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    #[test]
    fn rates_keep_a_third_decimal_only_when_there_is_one() {
        assert_eq!(fmt_rate(4.0), "$4.00");
        assert_eq!(fmt_rate(0.2), "$0.20");
        assert_eq!(fmt_rate(0.125), "$0.125");
        let haiku = price("claude-haiku-5-5").unwrap();
        assert_eq!(rates(&haiku), "Input $0.10 · Output $0.50 · Cache write $0.125 / $0.20 (1h) · Cache read $0.01 per MTok");
        assert_eq!(menu_rates(&price("claude-fable-5-2").unwrap()), "$8 / $40");
        assert_eq!(menu_rates(&haiku), "$0.10 / $0.50");
    }

    #[test]
    fn the_notes_explain_other_models_fast_calls_and_long_prompts() {
        use agent_session_core::model::{ModelUsage, Tokens};
        let tokens = Tokens { output: 1_000_000, ..Tokens::default() };
        let opus = ModelUsage { model: "claude-opus-5-5".into(), fast: true, long_prompt: true, responses: 3, tokens, cost_usd: None };
        let haiku = ModelUsage { model: "claude-haiku-4-5".into(), fast: false, long_prompt: false, responses: 2, tokens: Tokens::default(), cost_usd: None };
        let c = compare(&[opus, haiku], "claude-haiku-5-5").unwrap();
        let notes = notes(&c);
        assert_eq!(notes[0], "Haiku 4.5 keeps its price: 2 calls, $0.00, in both totals.");
        assert_eq!(notes[1], "3 calls ran in fast mode; Haiku 5.5 has none, so they are priced at its standard rate.");
        assert_eq!(notes[2], "3 calls had prompts over 100k, which Haiku 5.5 prices higher.");
        assert_eq!(notes.len(), 4);
    }
}
