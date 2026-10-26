//! Detail panel: header + Metadata / Conversation tabs, and the state of the Conversation tab.
//!
//! The conversation is a `list` of rows (`conversation::Row`), one per section of a turn card;
//! `DetailState` keeps that row table in step with what is open, and `turn_cards.rs` draws it.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use agent_session_core::model::{HitPlace, Part};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::*;

use crate::conversation::{Card, ConversationView, Row, StepBody, card_rows, row_of, rows};
use crate::model::SessionRow;
use crate::style::{self, FluentBuilder};
use crate::AppRoot;

pub const DETAIL_W: f32 = 580.;
/// Rows measured ahead of and behind the viewport, so a scroll does not pop rows in.
const LIST_OVERDRAW: f32 = 400.;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Metadata,
    Conversation,
}

/// Where the conversation of the selected session is.
#[derive(Clone, Debug, PartialEq)]
pub enum Load {
    /// Nothing selected, or a session without a transcript.
    Idle,
    /// Asked the reader thread; skeleton cards until it answers.
    Loading,
    Failed(SharedString),
    Ready,
}

pub struct DetailState {
    pub tab: DetailTab,
    pub load: Load,
    pub(crate) view: Option<Arc<ConversationView>>,
    /// What the list shows, one entry per list item.
    pub(crate) rows: Vec<Row>,
    pub(crate) list: ListState,
    /// Cards whose steps are listed.
    pub(crate) open_turns: HashSet<usize>,
    /// Tool and thinking steps shown open, `(card, step)`.
    pub(crate) open_steps: HashSet<(usize, usize)>,
    /// Folded texts the user opened (prompt, reply, process text, tool output), and compaction
    /// dividers showing their summary. A hit past a fold opens it by itself (`Folded::fold`).
    pub(crate) unfolded: HashSet<(usize, Part)>,
    /// The tick under the mouse: the stats line describes its step.
    pub(crate) hover_tick: Option<(usize, usize)>,
    /// The step last opened from its tick: its row is lit and its tick ringed.
    pub(crate) pointed: Option<(usize, usize)>,
    /// The hit place the view is parked on (index into `view.hits`).
    current_hit: Option<usize>,
    /// Set by a park, taken by the first frame that lays out the hit: that frame scrolls the hit
    /// itself into view (see `turn_cards::reveal_marker`).
    pub(crate) reveal: Rc<Cell<bool>>,
}

impl Default for DetailState {
    fn default() -> Self {
        Self {
            tab: DetailTab::Conversation,
            load: Load::Idle,
            view: None,
            rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            open_turns: HashSet::new(),
            open_steps: HashSet::new(),
            unfolded: HashSet::new(),
            hover_tick: None,
            pointed: None,
            current_hit: None,
            reveal: Rc::default(),
        }
    }
}

impl DetailState {
    /// Another session was selected: nothing of the last one stays on screen.
    pub fn begin_loading(&mut self) {
        self.forget();
        self.load = Load::Loading;
    }

    pub fn clear(&mut self) {
        self.forget();
        self.load = Load::Idle;
    }

    pub fn fail(&mut self, message: impl Into<SharedString>) {
        self.forget();
        self.load = Load::Failed(message.into());
    }

    fn forget(&mut self) {
        self.view = None;
        self.rows.clear();
        self.list.reset(0);
        self.open_turns.clear();
        self.open_steps.clear();
        self.unfolded.clear();
        self.hover_tick = None;
        self.pointed = None;
        self.current_hit = None;
        self.reveal.set(false);
    }

    /// Show a conversation, parked on its first search hit (or at the top without one).
    /// `keep_open` is for the same session read again with another search: what the user
    /// opened stays open.
    pub fn set_conversation(&mut self, view: Arc<ConversationView>, keep_open: bool) {
        if !keep_open {
            self.open_turns.clear();
            self.open_steps.clear();
            self.unfolded.clear();
            self.pointed = None;
        }
        self.hover_tick = None;
        self.current_hit = (!view.hits.is_empty()).then_some(0);
        self.rows = rows(&view, &self.open_turns);
        self.view = Some(view);
        self.load = Load::Ready;
        self.list.reset(self.rows.len());
        self.park();
    }

    /// A conversation is on screen (possibly while it is read again for another search).
    pub fn has_conversation(&self) -> bool {
        self.view.is_some()
    }

    /// `Matcher::pattern()` of the search the conversation on screen was marked for.
    pub fn pattern(&self) -> Option<&str> {
        self.view.as_ref().and_then(|v| v.pattern.as_deref())
    }

    pub fn toggle_turn(&mut self, card: usize) {
        if !self.open_turns.remove(&card) {
            self.open_turns.insert(card);
        }
        self.refresh_card(card);
    }

    pub fn toggle_step(&mut self, card: usize, step: usize) {
        if !self.opens(card, step) {
            return;
        }
        if !self.open_steps.remove(&(card, step)) {
            self.open_steps.insert((card, step));
        }
        self.remeasure(Row::Step(card, step));
    }

    pub fn toggle_fold(&mut self, card: usize, part: Part) {
        if !self.unfolded.remove(&(card, part)) {
            self.unfolded.insert((card, part));
        }
        self.remeasure(row_showing(card, part));
    }

    /// A click on a tick: list the turn's steps, open this one and scroll to it.
    pub fn point_at(&mut self, card: usize, step: usize) {
        self.pointed = Some((card, step));
        self.open_step(card, step);
        self.scroll_to(Row::Step(card, step));
    }

    /// Open every turn's step list, or close them all but the one holding the current hit
    /// (and close every step but the current hit's).
    pub fn expand_all(&mut self, open: bool) {
        let Some(view) = self.view.clone() else { return };
        let keep = match self.current_place() {
            Some(HitPlace { item, part: Part::Step(step) }) => Some((*item, Some(*step))),
            Some(place) => Some((place.item, None)),
            None => None,
        };
        if !open {
            let closed: Vec<(usize, usize)> = self.open_steps.iter().copied().filter(|&(card, step)| keep != Some((card, Some(step)))).collect();
            for (card, step) in closed {
                self.open_steps.remove(&(card, step));
                self.remeasure(Row::Step(card, step));
            }
        }
        for (ix, card) in view.cards.iter().enumerate() {
            let Card::Turn(turn) = card else { continue };
            let changed = if open {
                !turn.steps.is_empty() && self.open_turns.insert(ix)
            } else {
                keep.map(|(card, _)| card) != Some(ix) && self.open_turns.remove(&ix)
            };
            if changed {
                self.refresh_card(ix);
            }
        }
    }

    /// Every turn with steps lists them.
    pub fn all_open(&self) -> bool {
        let Some(view) = &self.view else { return false };
        view.cards.iter().enumerate().all(|(ix, card)| !matches!(card, Card::Turn(t) if !t.steps.is_empty()) || self.open_turns.contains(&ix))
    }

    /// Move to the next / previous hit place, wrapping around.
    pub fn step_hit(&mut self, forward: bool) {
        let (Some(current), Some(count)) = (self.current_hit, self.view.as_ref().map(|v| v.hits.len())) else { return };
        self.current_hit = Some(if forward { (current + 1) % count } else { (current + count - 1) % count });
        self.park();
    }

    /// (1-based position, count) of the hit place the view is parked on.
    pub fn hit_position(&self) -> Option<(usize, usize)> {
        Some((self.current_hit? + 1, self.view.as_ref()?.hits.len()))
    }

    pub fn current_place(&self) -> Option<&HitPlace> {
        self.view.as_ref()?.hits.get(self.current_hit?)
    }

    /// Scroll a card's first row to the top.
    pub fn scroll_to_card(&mut self, card: usize) {
        if let Some(ix) = self.rows.iter().position(|row| row.card() == card) {
            self.list.scroll_to(ListOffset { item_ix: ix, offset_in_item: px(0.) });
        }
    }

    /// Scroll the current hit place to the top, opening whatever hides it; back to the top
    /// without one.
    fn park(&mut self) {
        let Some(place) = self.current_place().cloned() else {
            self.list.scroll_to(ListOffset::default());
            self.reveal.set(false);
            return;
        };
        self.reveal.set(true);
        match place.part {
            Part::Step(step) => self.open_step(place.item, step),
            Part::Summary => {
                if self.unfolded.insert((place.item, Part::Summary)) {
                    self.remeasure(Row::Compact(place.item));
                }
            }
            // a hit past a fold opened it already (`Folded::fold`)
            Part::Prompt | Part::Reply => {}
        }
        if let Some(ix) = row_of(&self.rows, &place) {
            self.list.scroll_to(ListOffset { item_ix: ix, offset_in_item: px(0.) });
        }
    }

    /// List the turn's steps and open `step` (process text has nothing to open).
    fn open_step(&mut self, card: usize, step: usize) {
        let opened = self.opens(card, step) && self.open_steps.insert((card, step));
        if self.open_turns.insert(card) {
            self.refresh_card(card);
        } else if opened {
            self.remeasure(Row::Step(card, step));
        }
    }

    /// The step shows more when open: a tool call or a thinking.
    fn opens(&self, card: usize, step: usize) -> bool {
        let Some(Card::Turn(turn)) = self.view.as_ref().and_then(|v| v.cards.get(card)) else { return false };
        turn.steps.get(step).is_some_and(|s| !matches!(s.body, StepBody::Text(_)))
    }

    fn scroll_to(&mut self, row: Row) {
        if let Some(ix) = self.rows.iter().position(|r| *r == row) {
            self.list.scroll_to(ListOffset { item_ix: ix, offset_in_item: px(0.) });
        }
    }

    /// Put a card's rows in line with its open state. Only the rows that change are spliced,
    /// so the list keeps the measured heights and the scroll position of everything else.
    fn refresh_card(&mut self, card: usize) {
        let Some(view) = self.view.clone() else { return };
        let Some(start) = self.rows.iter().position(|row| row.card() == card) else { return };
        let old = self.rows[start..].iter().take_while(|row| row.card() == card).count();
        let new = card_rows(card, &view.cards[card], self.open_turns.contains(&card));
        let before = &self.rows[start..start + old];
        let same_start = before.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let same_end = before[same_start..].iter().rev().zip(new[same_start..].iter().rev()).take_while(|(a, b)| a == b).count();
        let (from, to) = (start + same_start, start + old - same_end);
        let inserted = &new[same_start..new.len() - same_end];
        self.list.splice(from..to, inserted.len());
        self.rows.splice(from..to, inserted.iter().copied());
    }

    /// A row's content changed height: measure it again on the next layout.
    fn remeasure(&mut self, row: Row) {
        if let Some(ix) = self.rows.iter().position(|r| *r == row) {
            self.list.remeasure_items(ix..ix + 1);
        }
    }
}

/// The row a folded part is drawn in.
fn row_showing(card: usize, part: Part) -> Row {
    match part {
        Part::Prompt => Row::Head(card),
        Part::Step(step) => Row::Step(card, step),
        Part::Reply => Row::Tail(card),
        Part::Summary => Row::Compact(card),
    }
}

fn mono(cx: &App) -> SharedString {
    style::mono(cx)
}

fn section_label(text: &'static str) -> Div {
    div().text_size(px(10.)).font_weight(FontWeight::SEMIBOLD).text_color(style::muted()).mb(px(4.)).child(text.to_uppercase())
}

fn section(title: &'static str, body: SharedString) -> Div {
    v_flex()
        .mt(px(12.))
        .pt(px(10.))
        .border_t_1()
        .border_color(style::hairline())
        .child(section_label(title))
        .child(div().text_size(px(12.)).line_height(px(16.)).child(body))
}

fn meta_pair(label: &'static str, value: AnyElement) -> Div {
    h_flex()
        .flex_1()
        .min_w_0()
        .h(px(22.))
        .items_center()
        .gap(px(8.))
        .child(div().w(px(64.)).flex_none().text_size(px(11.)).text_color(style::muted()).child(label))
        .child(div().flex_1().min_w_0().truncate().child(value))
}

fn mono_value(value: SharedString, cx: &App) -> Div {
    div().font_family(mono(cx)).text_size(px(11.)).truncate().child(value)
}

impl AppRoot {
    pub fn set_detail_tab(&mut self, tab: DetailTab, cx: &mut Context<Self>) {
        self.detail.tab = tab;
        cx.notify();
    }

    pub fn render_detail(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let Some(row) = self.selected.clone() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(style::muted())
                .child("Select a session")
                .into_any_element();
        };
        v_flex()
            .size_full()
            .min_h_0()
            .bg(style::white())
            .child(self.render_detail_header(&row, cx))
            .child(match self.detail.tab {
                DetailTab::Metadata => self.render_metadata(&row, cx).into_any_element(),
                DetailTab::Conversation => self.render_conversation(&row, cx).into_any_element(),
            })
            .into_any_element()
    }

    fn render_detail_header(&self, row: &Arc<SessionRow>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let dot = || div().text_color(style::dot_sep()).child("·");
        let summary = format!("{} prompts · {} tools · {}", row.prompts, row.tools, row.size_fmt);
        let tab_ix = match self.detail.tab {
            DetailTab::Metadata => 0,
            DetailTab::Conversation => 1,
        };
        v_flex()
            .flex_none()
            .pt(px(10.))
            .px(px(14.))
            .pb(px(8.))
            .border_b_1()
            .border_color(style::border_soft())
             .child(h_flex().gap(px(6.)).child(div().flex_1().truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).line_height(px(18.)).child(row.title.clone()))
                .child(Button::new("favorite-session").ghost().small().label(if row.favorite { "★" } else { "☆" }).text_color(style::favorite()).on_click(cx.listener(|this, _, window, cx| this.toggle_favorite(window, cx)))))
            .child(
                h_flex()
                    .mt(px(3.))
                    .gap(px(8.))
                    .items_center()
                    .font_family(mono(cx))
                    .text_size(px(11.))
                    .text_color(style::head_fg())
                    .child(row.short_id.clone())
                    .child(dot())
                    .child(row.project.clone())
                    .child(dot())
                    .child(row.branch.clone())
                    .children(style::kind_badge(row.kind, cx)),
            )
            .child(
                h_flex()
                    .mt(px(8.))
                    .gap(px(8.))
                    .items_center()
                    .child(
                        TabBar::new("detail-tabs")
                            .segmented()
                            .xsmall()
                            .selected_index(tab_ix)
                            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                let tab = if *ix == 0 { DetailTab::Metadata } else { DetailTab::Conversation };
                                this.set_detail_tab(tab, cx);
                            }))
                            .children(["Metadata", "Conversation"].map(|l| Tab::new().label(l))),
                    )
                    .child(div().flex_1())
                    .child(div().font_family(mono(cx)).text_size(px(11.)).text_color(style::muted()).child(summary)),
            )
    }

    fn render_metadata(&self, row: &Arc<SessionRow>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pair_row = |a, b| h_flex().w_full().gap(px(8.)).child(a).child(b);
        div()
            .id("metadata")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(
                v_flex()
                    .px(px(14.))
                    .pt(px(10.))
                    .pb(px(16.))
                    .child(meta_pair("Favorite", mono_value(row.favorite_path.clone(), cx).into_any_element()))
                    .child(pair_row(
                        meta_pair("Project", div().child(row.project.clone()).into_any_element()),
                        meta_pair("Branch", mono_value(row.branch.clone(), cx).into_any_element()),
                    ))
                    .child(meta_pair("Path", mono_value(row.project_path.clone(), cx).into_any_element()))
                    .child(meta_pair("ID", mono_value(row.id.clone(), cx).into_any_element()))
                    .child(pair_row(
                        meta_pair("Started", mono_value(row.started_full.clone(), cx).into_any_element()),
                        meta_pair("Updated", mono_value(row.updated_full.clone(), cx).into_any_element()),
                    ))
                    .child(pair_row(
                        meta_pair("Prompts", mono_value(row.prompts_fmt.clone(), cx).into_any_element()),
                        meta_pair("Tools", mono_value(row.tools_fmt.clone(), cx).into_any_element()),
                    ))
                    .child(pair_row(
                        meta_pair("Size", mono_value(row.size_fmt.clone(), cx).into_any_element()),
                        meta_pair(
                            "Cost",
                            h_flex()
                                .gap(px(8.))
                                .child(mono_value(row.cost_fmt.clone(), cx))
                                .child(mono_value(format!("CC: {}", row.cc_cost_fmt).into(), cx).text_color(style::muted()))
                                .into_any_element(),
                        ),
                    ))
                    .child(meta_pair("Tokens", mono_value(row.tokens_fmt.clone(), cx).into_any_element()))
                    .child(meta_pair(
                        "Context",
                        h_flex()
                            .gap(px(8.))
                            .child(mono_value(row.context_fmt.clone(), cx))
                            .child(mono_value(row.context_detail.clone(), cx).text_color(style::text_secondary()))
                            .into_any_element(),
                    ))
                    .when(!row.unpriced.is_empty(), |el| {
                        el.child(meta_pair("Unpriced", mono_value(row.unpriced.clone(), cx).text_color(style::muted()).into_any_element()))
                    })
                    .child(pair_row(
                        meta_pair("Messages", mono_value(row.messages_fmt.clone(), cx).into_any_element()),
                        meta_pair("Duration", mono_value(row.duration_fmt.clone(), cx).into_any_element()),
                    ))
                    .child(pair_row(
                        meta_pair("Kind", div().child(row.kind.label()).into_any_element()),
                        meta_pair("Subagents", mono_value(row.subagents_fmt.clone(), cx).into_any_element()),
                    ))
                    .child(pair_row(
                        meta_pair("Mode", mono_value(row.permission_mode.clone(), cx).into_any_element()),
                        meta_pair("Version", mono_value(row.version.clone(), cx).into_any_element()),
                    ))
                    .child(meta_pair("Models", mono_value(row.models.clone(), cx).into_any_element()))
                    .child(meta_pair("Top tools", mono_value(row.top_tools.clone(), cx).into_any_element()))
                    .child(meta_pair("PR", mono_value(row.pr_url.clone(), cx).into_any_element()))
                    .child(meta_pair("Resume", mono_value(row.resume.clone(), cx).text_color(style::text_secondary()).into_any_element()))
                    .child(section("First prompt", row.first_prompt_full.clone()))
                    .child(section("Last prompt", row.last_prompt_full.clone()))
                    .child(section("Last message", row.last_message_full.clone())),
            )
    }
}

#[cfg(test)]
mod tests {
    // `gpui_kit::*` exports gpui's own `test` attribute; name the standard one explicitly.
    use core::prelude::v1::test;

    use agent_session_core::model::{Compaction, ConvItem, Marked, Reply, Step, StepKind, ToolCall, ToolResult, Turn};

    use super::*;
    use crate::conversation::build;
    use crate::conversation::tests::{conv, marked, says, text, thinks, today, tool, turn};

    fn mark(t: &str, needle: Option<&str>) -> Marked {
        needle.map_or_else(|| text(t), |needle| marked(t, needle))
    }

    /// A Bash call whose output is `out`, marked for `needle`.
    fn run(out: &str, needle: Option<&str>) -> Step {
        let output = mark(out, needle);
        let result = ToolResult { bytes: out.len(), lines: output.lines, output, is_error: false };
        Step { ts: None, kind: StepKind::Tool(ToolCall { name: "Bash".into(), arg: "cargo test".into(), input: Vec::new(), result: Some(result) }) }
    }

    /// Card 0: TURN 1 with two steps; card 1: a compaction; card 2: TURN 2 with thinking, a Bash
    /// call and an edit. Read with `needle`, it is marked in TURN 1's prompt ("fix the parser"),
    /// TURN 2's prompt ("and the tests?"), the Bash output ("test parser ... ok") and TURN 2's
    /// reply ("the parser is fixed").
    fn session(needle: Option<&str>) -> Arc<ConversationView> {
        let first = Turn {
            prompt: mark("fix the parser", needle),
            ..turn(Some(1), None, "", vec![tool("Read", "src/parse.rs", Some(("fn parse()", false))), says("on it")])
        };
        let second = Turn {
            prompt: mark("and the tests?", needle),
            reply: Some(Reply { ts: None, text: mark("the parser is fixed", needle) }),
            ..turn(Some(2), None, "", vec![thinks("let me run them"), run("test parser ... ok", needle), tool("Edit", "src/parse.rs", Some(("ok", false)))])
        };
        let compaction = Compaction { ts: None, summary: Some(text("so far")) };
        let mut view = build(&conv(vec![ConvItem::Turn(first), ConvItem::Compact(compaction), ConvItem::Turn(second)]), "/p", today());
        view.pattern = needle.map(Into::into);
        Arc::new(view)
    }

    fn place(item: usize, part: Part) -> Option<HitPlace> {
        Some(HitPlace { item, part })
    }

    fn top_row(d: &DetailState) -> Row {
        d.rows[d.list.logical_scroll_top().item_ix]
    }

    const CLOSED: [Row; 5] = [Row::Head(0), Row::Tail(0), Row::Compact(1), Row::Head(2), Row::Tail(2)];

    #[test]
    fn the_view_parks_on_the_first_hit_and_steps_around_the_others() {
        let mut d = DetailState::default();
        d.set_conversation(session(Some("fix")), false);
        assert_eq!(d.hit_position(), Some((1, 2)));
        assert_eq!(d.current_place().cloned(), place(0, Part::Prompt));
        d.step_hit(true);
        assert_eq!((d.hit_position(), d.current_place().cloned()), (Some((2, 2)), place(2, Part::Reply)));
        d.step_hit(true);
        assert_eq!(d.hit_position(), Some((1, 2)), "past the last hit comes the first again");
        d.step_hit(false);
        assert_eq!(d.hit_position(), Some((2, 2)), "and the other way round");
    }

    #[test]
    fn a_new_search_keeps_open_turns_and_parks_on_its_first_hit() {
        let mut d = DetailState::default();
        d.set_conversation(session(None), false);
        d.toggle_turn(0);
        d.toggle_step(0, 0);
        d.set_conversation(session(Some("tests")), true);
        assert!(d.open_turns.contains(&0) && d.open_steps.contains(&(0, 0)), "what was open stays open");
        assert_eq!(d.current_place().cloned(), place(2, Part::Prompt));
        assert_eq!(top_row(&d), Row::Head(2));
        assert_eq!(d.pattern(), Some("tests"));
    }

    #[test]
    fn no_search_marks_nothing_and_parks_at_the_top() {
        let mut d = DetailState::default();
        d.set_conversation(session(Some("fixed")), false);
        assert_eq!(top_row(&d), Row::Tail(2), "parked on the reply");
        d.set_conversation(session(None), true);
        assert_eq!(d.hit_position(), None);
        assert!(d.view.as_ref().unwrap().hits.is_empty());
        d.step_hit(true);
        assert_eq!(d.hit_position(), None, "nothing to step through");
        assert_eq!(d.list.logical_scroll_top().item_ix, 0, "back at the top");
    }

    #[test]
    fn stepping_to_a_hit_in_a_closed_turn_opens_the_turn_and_the_step() {
        let mut d = DetailState::default();
        d.set_conversation(session(Some("parser")), false);
        assert_eq!(d.hit_position(), Some((1, 3)));
        assert!(d.open_turns.is_empty(), "a hit in a prompt opens nothing");
        d.step_hit(true);
        assert_eq!(d.current_place().cloned(), place(2, Part::Step(1)));
        assert!(d.open_turns.contains(&2), "the turn lists its steps");
        assert_eq!(d.open_steps, HashSet::from([(2, 1)]), "and the step with the hit is open");
        assert_eq!(&d.rows[3..], [Row::Head(2), Row::Step(2, 0), Row::Step(2, 1), Row::Step(2, 2), Row::Tail(2)]);
        assert_eq!(d.list.item_count(), d.rows.len());
    }

    #[test]
    fn parking_scrolls_the_list_to_the_row_of_the_hit() {
        let mut d = DetailState::default();
        d.set_conversation(session(Some("parser")), false);
        assert_eq!(top_row(&d), Row::Head(0));
        d.step_hit(true);
        let top = d.list.logical_scroll_top();
        assert_eq!((d.rows[top.item_ix], top.offset_in_item), (Row::Step(2, 1), px(0.)), "the step itself, not its card");
        d.step_hit(true);
        assert_eq!(top_row(&d), Row::Tail(2));
        d.step_hit(true);
        assert_eq!(top_row(&d), Row::Head(0), "around again");
    }

    #[test]
    fn opening_a_turn_splices_its_steps_in_and_leaves_the_rows_above_alone() {
        let mut d = DetailState::default();
        d.set_conversation(session(None), false);
        assert_eq!(d.rows, CLOSED);
        d.list.scroll_to(ListOffset { item_ix: 3, offset_in_item: px(5.) });

        d.toggle_turn(2);
        assert_eq!(&d.rows[..4], &CLOSED[..4], "the rows above are untouched");
        assert_eq!(&d.rows[4..], [Row::Step(2, 0), Row::Step(2, 1), Row::Step(2, 2), Row::Tail(2)]);
        assert_eq!(d.list.item_count(), d.rows.len());
        let top = d.list.logical_scroll_top();
        assert_eq!((d.rows[top.item_ix], top.offset_in_item), (Row::Head(2), px(5.)), "the list stays where it was");

        d.toggle_turn(0);
        assert_eq!(&d.rows[..4], [Row::Head(0), Row::Step(0, 0), Row::Step(0, 1), Row::Tail(0)]);
        assert_eq!(d.list.item_count(), d.rows.len());
        assert_eq!(top_row(&d), Row::Head(2), "rows put in above the viewport move it along");

        d.toggle_turn(0);
        d.toggle_turn(2);
        assert_eq!(d.rows, CLOSED);
        assert_eq!((d.list.item_count(), top_row(&d)), (CLOSED.len(), Row::Head(2)));
    }

    #[test]
    fn expand_all_opens_every_turn_and_collapse_all_keeps_the_current_hit_open() {
        let mut d = DetailState::default();
        d.set_conversation(session(Some("parser")), false);
        assert!(!d.all_open());
        d.expand_all(true);
        assert!(d.all_open());
        assert_eq!(d.open_turns, HashSet::from([0, 2]));
        assert_eq!(d.rows.len(), 10);
        assert_eq!(d.list.item_count(), d.rows.len());

        d.step_hit(true);
        d.toggle_step(0, 0);
        d.toggle_step(2, 0);
        d.expand_all(false);
        assert_eq!(d.open_turns, HashSet::from([2]), "the turn with the current hit stays open");
        assert_eq!(d.open_steps, HashSet::from([(2, 1)]), "and so does its step, alone");
        assert!(!d.all_open());
        assert_eq!(d.rows, [Row::Head(0), Row::Tail(0), Row::Compact(1), Row::Head(2), Row::Step(2, 0), Row::Step(2, 1), Row::Step(2, 2), Row::Tail(2)]);
        assert_eq!(d.list.item_count(), d.rows.len());

        d.set_conversation(session(None), true);
        d.expand_all(false);
        assert_eq!(d.rows, CLOSED, "without a hit everything closes");
    }

    #[test]
    fn a_new_session_starts_folded() {
        let mut d = DetailState::default();
        d.set_conversation(session(None), false);
        d.toggle_turn(2);
        d.toggle_step(2, 1);
        d.toggle_fold(0, Part::Prompt);
        d.toggle_fold(1, Part::Summary);
        d.point_at(0, 0);
        d.begin_loading();
        assert_eq!(d.load, Load::Loading);
        assert!(!d.has_conversation() && d.rows.is_empty() && d.list.item_count() == 0, "nothing of the last session stays on screen");

        d.set_conversation(session(None), false);
        assert_eq!(d.load, Load::Ready);
        assert!(d.open_turns.is_empty() && d.open_steps.is_empty() && d.unfolded.is_empty() && d.pointed.is_none());
        assert_eq!(d.rows, CLOSED);
    }

    #[test]
    fn a_tick_click_lists_the_steps_opens_its_step_and_scrolls_to_it() {
        let mut d = DetailState::default();
        d.set_conversation(session(None), false);
        d.point_at(2, 1);
        assert!(d.open_turns.contains(&2) && d.open_steps.contains(&(2, 1)));
        assert_eq!((d.pointed, top_row(&d)), (Some((2, 1)), Row::Step(2, 1)));
        d.point_at(0, 1);
        assert!(!d.open_steps.contains(&(0, 1)), "process text is never collapsed, so there is nothing to open");
        assert_eq!(top_row(&d), Row::Step(0, 1));
    }
}
