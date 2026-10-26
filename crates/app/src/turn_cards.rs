//! The Conversation tab: a top bar (summary, hit navigation, Expand all) over a `list` of
//! turn-card rows. A row is one section of a card (`conversation::Row`): the head draws the
//! card's top edge, the tail its bottom edge and steps only its sides, so the rows put together
//! look like one card while the list lays out only the rows on screen.
//!
//! Every string comes pre-formatted in `ConversationView`; what is open, folded or hovered is
//! `DetailState`'s. Sizes and colors follow the M4 mockups 1–5.

use std::ops::Range;

use agent_session_core::model::{HitPlace, Part};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::conversation::{Card, CompactCard, Folded, InputKind, Row, RunStyle, Segment, Status, StepBody, StepView, Tail, TickKind, TurnCard, text_runs};
use crate::detail::{DetailTab, Load};
use crate::style::{self, FluentBuilder};
use crate::AppRoot;

/// Time / label column left of prompts, replies and step rows.
const LABEL_W: f32 = 58.;
/// Space between a card and the panel's edges.
const CARD_X: f32 = 10.;
/// Where the status line, the ticks and open step boxes start: past the label column.
const BODY_X: f32 = 76.;
const CARD_RADIUS: f32 = 6.;
const STEP_ROW_H: f32 = 22.;
/// Tool name column of a step row (the mockup's width), and how far a long name may widen it.
const TOOL_NAME_W: f32 = 56.;
const TOOL_NAME_MAX_W: f32 = 168.;

const HISTORY_NOTE: &str = "Claude Code cleaned up this session's transcript. Only its prompts in history.jsonl are left: no replies, tool calls or cost.";

/// Ids of elements inside rows: unique per card and step, whatever row the list puts them in.
fn element_id(name: &'static str, card: usize, ix: usize) -> ElementId {
    ElementId::NamedInteger(name.into(), ((card as u64) << 32) | ix as u64)
}

/// `Part` as the low half of an `element_id`.
fn part_ix(part: Part) -> usize {
    match part {
        Part::Prompt => 0,
        Part::Reply => 1,
        Part::Summary => 2,
        Part::Step(step) => 3 + step,
    }
}

/// A folded text as it shows now: folded (up to the fold, or a notification's gist), or whole
/// once opened; with the hits of what shows.
fn folded_text(text: &Folded, open: bool, marks: HighlightStyle) -> StyledText {
    let (shown, hits) = if open { (&text.text, &text.hits) } else { (&text.short, &text.short_hits) };
    StyledText::new(shown.clone()).with_highlights(hits.iter().map(|r| (r.clone(), marks)))
}

/// Segments that are the whole text, as it is: nothing to cut.
fn is_plain(segments: &[Segment], len: usize) -> bool {
    match segments {
        [] => true,
        [Segment::Text { range, inline_code }] => range.len() == len && inline_code.is_empty(),
        _ => false,
    }
}

/// A stretch of `text_runs`: a hit is marked (over inline code's gray), inline code is gray and
/// its backticks are not drawn. Every stretch of inline code also gets the mono font, through
/// the font overrides of its `StyledText`.
fn run_highlight(run: RunStyle, marks: HighlightStyle) -> HighlightStyle {
    if run.hit {
        return marks;
    }
    // a highlight's color is blended over the text's, so a transparent one changes nothing:
    // fading out all the way hides the backtick
    HighlightStyle { background_color: Some(style::inline_code_bg()), fade_out: run.backtick.then_some(1.), ..Default::default() }
}

/// A fenced code block (mockup 2's `.block`): mono on light gray in a thin frame; lines never
/// wrap, what is too wide is cut off. `after` / `before`: text comes before / after it.
fn code_block(text: StyledText, after: bool, before: bool, cx: &App) -> Div {
    div()
        .when(after, |el| el.mt(px(6.)))
        .when(before, |el| el.mb(px(6.)))
        .min_w_0()
        .overflow_hidden()
        .px(px(8.))
        .py(px(6.))
        .bg(style::code_block_bg())
        .border_1()
        .border_color(style::hairline())
        .rounded(px(4.))
        .font_family(style::mono(cx))
        .font_weight(FontWeight::NORMAL)
        .text_size(px(11.))
        .line_height(px(15.))
        .text_color(style::asst_text())
        .whitespace_nowrap()
        .child(text)
}

/// `[` `]` jump between turns, in key caps (mockup 2's `.kbd`).
fn turn_keys_hint(cx: &App) -> Div {
    let key = |glyph: &'static str| {
        div()
            .min_w(px(14.))
            .px(px(3.))
            .flex()
            .justify_center()
            .border_1()
            .border_b_2()
            .border_color(style::control_border())
            .rounded(px(3.))
            .bg(style::white())
            .font_family(style::mono(cx))
            .text_size(px(9.5))
            .line_height(px(12.))
            .text_color(style::head_fg())
            .child(glyph)
    };
    h_flex().flex_none().items_center().gap(px(3.)).text_color(style::muted()).child(key("[")).child(key("]")).child(div().ml(px(1.)).child("jump between turns"))
}

/// Above the cards of a history-only session (mockup 5's `.note`): where its replies went.
fn history_note() -> Div {
    div().flex_none().bg(style::conversation_bg()).px(px(CARD_X)).pt(px(8.)).child(
        div()
            .px(px(10.))
            .py(px(7.))
            .bg(style::head_bg())
            .border_1()
            .border_dashed()
            .border_color(style::control_border())
            .rounded(px(CARD_RADIUS))
            .text_size(px(11.))
            .line_height(px(15.))
            .text_color(style::head_fg())
            .child(HISTORY_NOTE),
    )
}

/// A row's slice of a card: the sides always, the top edge on the head, the bottom one on the tail.
fn card_slice(top: bool, bottom: bool) -> Div {
    v_flex()
        .bg(style::white())
        .border_color(style::card_border())
        .border_l_1()
        .border_r_1()
        .when(top, |el| el.border_t_1().rounded_t(px(CARD_RADIUS)))
        .when(bottom, |el| el.border_b_1().rounded_b(px(CARD_RADIUS)))
}

fn row_frame(top: bool) -> Div {
    v_flex().w_full().px(px(CARD_X)).when(top, |el| el.pt(px(8.)))
}

/// `ASST 10:39` and the like: the label column of a card section.
fn label_column(label: Option<SharedString>, time: SharedString, label_fg: Hsla, time_fg: Hsla, cx: &App) -> Div {
    v_flex()
        .w(px(LABEL_W))
        .flex_none()
        .font_family(style::mono(cx))
        .text_size(px(10.))
        .line_height(px(14.))
        .children(label.map(|label| div().font_weight(FontWeight::BOLD).text_color(label_fg).child(label)))
        .child(div().text_color(time_fg).child(time))
}

fn hits_badge(text: SharedString, cx: &App) -> Div {
    div()
        .flex_none()
        .font_family(style::mono(cx))
        .text_size(px(10.))
        .line_height(px(14.))
        .px(px(4.))
        .rounded(px(3.))
        .bg(style::hit_bg())
        .text_color(style::text())
        .child(text)
}

/// The gray line at the end of a text core kept only part of: `cut · showing 3,000 of 14,212 chars`.
fn cut_line(text: SharedString) -> Div {
    div().mt(px(2.)).text_size(px(10.)).line_height(px(13.)).text_color(style::muted_light()).child(text)
}

/// `cut_line` under a folded text, once it shows whole (opened, or never folded).
fn cut_note(text: &Folded, open: bool) -> Option<Div> {
    (!text.cut.is_empty() && (open || text.fold.is_none())).then(|| cut_line(text.cut.clone()))
}

fn ring(color: Hsla, spread: f32) -> BoxShadow {
    BoxShadow { color, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(spread), inset: false }
}

/// A note in place of the cards (history session, read error, nothing to show).
fn conversation_note(text: SharedString, color: Hsla) -> Div {
    div().flex_1().min_h_0().bg(style::conversation_bg()).px(px(12.)).py(px(10.)).text_size(px(11.)).text_color(color).child(text)
}

/// Placeholder cards while the transcript is read.
fn skeleton() -> Div {
    let bar = |w: DefiniteLength, h: f32| Skeleton::new().w(w).h(px(h));
    v_flex().flex_1().min_h_0().bg(style::conversation_bg()).children((0..3).map(move |_| {
        div().px(px(CARD_X)).pt(px(8.)).child(
            v_flex()
                .gap(px(8.))
                .p(px(12.))
                .bg(style::white())
                .border_1()
                .border_color(style::card_border())
                .rounded(px(CARD_RADIUS))
                .child(bar(px(96.).into(), 10.))
                .child(bar(relative(1.), 10.))
                .child(bar(relative(0.7), 10.))
                .child(Skeleton::new().secondary().w(relative(0.5)).h(px(8.))),
        )
    }))
}

impl AppRoot {
    /// The bar, the cards; for a history-only session (`3 prompts from history.jsonl`) a note
    /// between them says where its replies went.
    pub(crate) fn render_conversation(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let body = match (&self.detail.load, &self.detail.view) {
            (Load::Failed(message), _) => conversation_note(message.clone(), style::error_text()).into_any_element(),
            (Load::Ready, Some(view)) if view.cards.is_empty() => conversation_note("No messages in this session".into(), style::muted_light()).into_any_element(),
            (Load::Ready, Some(_)) => self.render_list(cx).into_any_element(),
            _ => skeleton().into_any_element(),
        };
        let note = self.detail.view.as_ref().is_some_and(|v| v.history).then(history_note);
        v_flex().flex_1().min_h_0().child(self.render_conversation_bar(cx)).children(note).child(body)
    }

    /// `9 turns · 214 tool calls`, the `[ ]` hint or `▲ 3 / 9 hits ▼`, Expand all.
    fn render_conversation_bar(&self, cx: &mut Context<Self>) -> Div {
        let view = self.detail.view.as_ref();
        let has_steps = view.is_some_and(|v| v.cards.iter().any(|c| matches!(c, Card::Turn(t) if !t.steps.is_empty())));
        let all_open = self.detail.all_open();
        let hit_position = self.detail.hit_position();
        // a search's hit navigation takes the place of the hint
        let hint = hit_position.is_none() && view.is_some_and(|v| v.cards.iter().any(|c| matches!(c, Card::Turn(_))));
        h_flex()
            .flex_none()
            .h(px(28.))
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .bg(style::head_bg())
            .border_b_1()
            .border_color(style::hairline())
            .text_size(px(11.))
            .text_color(style::text_secondary())
            .child(div().font_family(style::mono(cx)).child(view.map(|v| v.summary.clone()).unwrap_or_default()))
            .when(hint, |el| el.child(turn_keys_hint(cx)))
            .child(div().flex_1())
            .children(hit_position.map(|(at, of)| self.render_hit_nav(at, of, cx)))
            .when(has_steps, |el| {
                el.child(
                    div()
                        .id("toggle-all")
                        .text_color(style::accent())
                        .cursor_default()
                        .hover(|el| el.text_decoration_1())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.detail.expand_all(!all_open);
                            cx.notify();
                        }))
                        .child(if all_open { "Collapse all" } else { "Expand all" }),
                )
            })
    }

    /// `▲ 2 / 9 hits ▼`: step through the hit places (also `n` / `shift-n`, `cmd-g`).
    fn render_hit_nav(&self, at: usize, of: usize, cx: &mut Context<Self>) -> Div {
        let step = |id: &'static str, glyph: &'static str, forward: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .w(px(16.))
                .h(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .text_size(px(8.))
                .text_color(style::text_secondary())
                .cursor_default()
                .hover(|el| el.bg(style::control_hover()))
                .on_click(cx.listener(move |this, _, _, cx| this.step_hit(forward, cx)))
                .child(glyph)
        };
        h_flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .mr(px(6.))
            .child(step("hit-prev", "▲", false, cx))
            .child(div().font_family(style::mono(cx)).text_size(px(10.)).child(format!("{at} / {of} hits")))
            .child(step("hit-next", "▼", true, cx))
    }

    pub fn step_hit(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.detail.step_hit(forward);
        cx.notify();
    }

    /// `[` / `]`: only while the Conversation tab is on screen.
    pub fn step_turn(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.detail_visible && self.detail.tab == DetailTab::Conversation {
            self.detail.step_turn(forward);
            cx.notify();
        }
    }

    fn render_list(&self, cx: &mut Context<Self>) -> Div {
        let view = cx.entity();
        // `list` calls back with `&mut App`: rows are rendered through the entity
        let rows = list(self.detail.list.clone(), move |ix, window, cx| view.update(cx, |this, cx| this.render_row(ix, window, cx))).size_full().pb(px(10.));
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .bg(style::conversation_bg())
            .child(rows)
            .vertical_scrollbar(&self.detail.list)
            .children(self.first_frame_probe(cx))
    }

    /// Paints after the rows of a session's first frame with content and reports the time it
    /// took since the row was selected (`detail:` line, see `report_first_frame`).
    fn first_frame_probe(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        self.detail_timing.as_ref()?.loaded?;
        let view = cx.entity().downgrade();
        Some(canvas(|_, _, _| {}, move |_, _, _, cx| view.update(cx, |this, _| this.report_first_frame()).unwrap_or(())).absolute().size(px(1.)))
    }

    fn render_row(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (Some(view), Some(&row)) = (self.detail.view.clone(), self.detail.rows.get(ix)) else { return div().into_any_element() };
        match (row, view.cards.get(row.card())) {
            (Row::Head(c), Some(Card::Turn(t))) => self.render_head(c, t, cx).into_any_element(),
            (Row::Step(c, s), Some(Card::Turn(t))) => self.render_step(c, s, t, cx).into_any_element(),
            (Row::Tail(c), Some(Card::Turn(t))) => self.render_tail(c, t, cx).into_any_element(),
            (Row::Compact(c), Some(Card::Compact(k))) => self.render_compact(c, k, cx).into_any_element(),
            _ => div().into_any_element(),
        }
    }

    /// Highlight style of the hits in `part` of card `card`: the current hit place is orange.
    fn marks(&self, card: usize, part: Part) -> HighlightStyle {
        if self.is_current(card, part) { style::hit_current_highlight() } else { style::hit_highlight() }
    }

    fn is_current(&self, card: usize, part: Part) -> bool {
        self.detail.current_place() == Some(&HitPlace { item: card, part })
    }

    /// A folded text of `part`, its hits marked. A prompt, process text or reply shows in
    /// segments: code blocks in a box, inline code in mono on gray.
    fn marked(&self, text: &Folded, open: bool, card: usize, part: Part, cx: &App) -> Div {
        let (shown, hits, segments) = if open { (&text.text, &text.hits, &text.segments) } else { (&text.short, &text.short_hits, &text.short_segments) };
        let (marks, current) = (self.marks(card, part), self.is_current(card, part));
        if is_plain(segments, shown.len()) {
            return self.revealing(folded_text(text, open, marks), hits.first().map(|hit| hit.start), current);
        }
        let mono = style::mono(cx);
        let mut pieces = Vec::with_capacity(segments.len());
        // the first piece's layout is where the text starts; the hit's, where in it the hit is
        let mut layouts: Option<(TextLayout, Option<(TextLayout, usize)>)> = None;
        for (ix, segment) in segments.iter().enumerate() {
            let range = match segment {
                Segment::Text { range, .. } | Segment::Code { range } => range.clone(),
            };
            let piece_hits: Vec<Range<usize>> =
                hits.iter().filter(|h| h.start < range.end && h.end > range.start).map(|h| h.start.max(range.start) - range.start..h.end.min(range.end) - range.start).collect();
            let piece = SharedString::from(shown[range.clone()].to_string());
            let styled = match segment {
                Segment::Text { inline_code, .. } => {
                    let code: Vec<Range<usize>> = inline_code.iter().map(|r| r.start - range.start..r.end - range.start).collect();
                    let runs = text_runs(&code, &piece_hits);
                    StyledText::new(piece).with_highlights(runs.into_iter().map(|(r, run)| (r, run_highlight(run, marks)))).with_font_family_overrides(code.into_iter().map(|r| (r, mono.clone())))
                }
                Segment::Code { .. } => StyledText::new(piece).with_highlights(piece_hits.iter().map(|r| (r.clone(), marks))),
            };
            let (_, hit) = layouts.get_or_insert_with(|| (styled.layout().clone(), None));
            if hit.is_none() && let Some(first) = piece_hits.first() {
                *hit = Some((styled.layout().clone(), first.start));
            }
            pieces.push(match segment {
                Segment::Text { .. } => div().child(styled),
                Segment::Code { .. } => code_block(styled, ix > 0, ix + 1 < segments.len(), cx),
            });
        }
        let marker = match layouts {
            Some((top, Some((layout, at)))) if current && self.detail.reveal.get() => Some(reveal_marker(self.detail.reveal.clone(), top, layout, at)),
            _ => None,
        };
        v_flex().relative().children(pieces).children(marker)
    }

    /// `text` in a block. Parking scrolls to the top of the row with the hit, which a long
    /// prompt or output can leave far below; so while a park is still to be shown, a block of
    /// the current hit place gets a marker that asks the list to bring the hit near the top.
    fn revealing(&self, text: StyledText, first_hit: Option<usize>, current: bool) -> Div {
        let marker = first_hit.filter(|_| current && self.detail.reveal.get()).map(|at| reveal_marker(self.detail.reveal.clone(), text.layout().clone(), text.layout().clone(), at));
        div().relative().child(text).children(marker)
    }

    /// `Show all · 18 lines` under a folded text, `Show less` once it is open.
    fn fold_link(&self, text: &Folded, card: usize, part: Part, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        text.fold?;
        let open = self.detail.unfolded.contains(&(card, part));
        Some(
            div()
                .id(element_id("fold", card, part_ix(part)))
                .mt(px(3.))
                .text_size(px(11.))
                .line_height(px(14.))
                .text_color(style::accent())
                .cursor_default()
                .hover(|el| el.text_decoration_1())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.detail.toggle_fold(card, part);
                    cx.notify();
                }))
                .child(if open { "Show less".into() } else { text.more.clone() }),
        )
    }

    /// Title, prompt, attachments; then the status line, ticks and stats.
    fn render_head(&self, c: usize, t: &TurnCard, cx: &mut Context<Self>) -> Div {
        let (bg, border, label_fg, time_fg) = if t.auto {
            (style::auto_bg(), style::hairline(), style::head_fg(), style::muted_light())
        } else {
            (style::user_bg(), style::user_border(), style::accent(), style::user_time())
        };
        let prompt_open = self.detail.unfolded.contains(&(c, Part::Prompt));
        let prompt = v_flex()
            .flex_1()
            .min_w_0()
            .when(t.auto && !t.origin.is_empty(), |el| {
                el.child(div().font_family(style::mono(cx)).text_size(px(10.)).line_height(px(14.)).text_color(style::muted()).child(t.origin.clone()))
            })
            .when(!t.prompt.text.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .map(|el| if t.auto { el.text_color(style::text_secondary()) } else { el.font_weight(FontWeight::MEDIUM) })
                        .child(self.marked(&t.prompt, prompt_open, c, Part::Prompt, cx)),
                )
            })
            .children(cut_note(&t.prompt, prompt_open))
            .children(self.fold_link(&t.prompt, c, Part::Prompt, cx))
            .when(!t.attachments.is_empty(), |el| {
                el.child(h_flex().flex_wrap().gap(px(4.)).mt(px(4.)).children(t.attachments.iter().map(|label| {
                    div()
                        .font_family(style::mono(cx))
                        .text_size(px(10.))
                        .line_height(px(16.))
                        .px(px(5.))
                        .rounded(px(3.))
                        .bg(style::white())
                        .border_1()
                        .border_color(style::user_border())
                        .text_color(style::text_secondary())
                        .child(label.clone())
                })))
            });
        // a prompt kept in history.jsonl is the whole card (mockup 5)
        let only = t.prompt_only;
        row_frame(true).child(
            card_slice(true, only)
                .child(
                    h_flex()
                        .items_start()
                        .gap(px(8.))
                        .pl(px(10.))
                        .pr(px(12.))
                        .py(px(7.))
                        .bg(bg)
                        .rounded_t(px(CARD_RADIUS - 1.))
                        .map(|el| if only { el.rounded_b(px(CARD_RADIUS - 1.)) } else { el.border_b_1().border_color(border) })
                        .child(label_column(Some(t.label.clone()), t.time.clone(), label_fg, time_fg, cx))
                        .child(prompt),
                )
                .when(!only, |el| el.child(self.render_status(c, t, cx))),
        )
    }

    /// `● 29 steps · 14m 48s · 1 error   2 hits   Show steps ▸`, the ticks and the stats line.
    fn render_status(&self, c: usize, t: &TurnCard, cx: &mut Context<Self>) -> Div {
        let open = self.detail.open_turns.contains(&c);
        let lamp = match t.status {
            Status::Ok => style::turn_ok(),
            Status::Failed => style::turn_err(),
            Status::Off => style::turn_off(),
        };
        let note_fg = if t.status == Status::Off { style::muted() } else { style::error_text() };
        let parts = [(&t.meta, style::text_secondary()), (&t.errors, style::error_text()), (&t.end_note, note_fg)];
        let mut said = h_flex().min_w_0().whitespace_nowrap();
        for (ix, (text, color)) in parts.into_iter().filter(|(text, _)| !text.is_empty()).enumerate() {
            if ix > 0 {
                said = said.child(" · ");
            }
            said = said.child(div().text_color(color).child(text.clone()));
        }
        // nothing but the card's bottom edge follows: the tail adds the padding
        let ends_here = !open && matches!(t.tail, Tail::Empty);
        v_flex()
            .pl(px(BODY_X))
            .pr(px(12.))
            .pt(px(6.))
            .pb(px(if ends_here { 1. } else { 7. }))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.))
                    .font_family(style::mono(cx))
                    .text_size(px(10.5))
                    .line_height(px(14.))
                    .text_color(style::text_secondary())
                    .child(div().size(px(7.)).flex_none().rounded_full().bg(lamp))
                    .child(said)
                    .when(t.hits > 0, |el| el.child(hits_badge(t.hits_fmt.clone(), cx)))
                    .child(div().flex_1())
                    .when(!t.steps.is_empty(), |el| {
                        el.child(
                            div()
                                .id(element_id("steps-toggle", c, 0))
                                .flex_none()
                                .font_family(style::sans(cx))
                                .text_size(px(11.))
                                .text_color(style::accent())
                                .cursor_default()
                                .hover(|el| el.text_decoration_1())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.detail.toggle_turn(c);
                                    cx.notify();
                                }))
                                .child(if open { "Hide steps ▾" } else { "Show steps ▸" }),
                        )
                    }),
            )
            .when(!t.steps.is_empty(), |el| el.child(self.render_ticks(c, t, cx)).child(self.render_stats(c, t, cx)))
    }

    /// One tick per step: a bar for a tool call (red when it failed), a dot for process text, a
    /// ring for thinking. Rings around: blue for the step under the mouse or last clicked, orange
    /// for the current hit, yellow for other hits. Each sits in a wider transparent cell that
    /// takes the hover and the click.
    fn render_ticks(&self, c: usize, t: &TurnCard, cx: &mut Context<Self>) -> Div {
        let current = match self.detail.current_place() {
            Some(HitPlace { item, part: Part::Step(step) }) if *item == c => Some(*step),
            _ => None,
        };
        h_flex().flex_wrap().items_center().mt(px(4.)).mb(px(3.)).children(t.ticks.iter().enumerate().map(|(s, tick)| {
            let pointed = self.detail.hover_tick == Some((c, s)) || self.detail.pointed == Some((c, s));
            let rings = if pointed {
                vec![ring(style::accent(), 2.5), ring(style::white(), 1.)]
            } else if current == Some(s) {
                vec![ring(style::hit_current_bg(), 2.)]
            } else if tick.hit {
                vec![ring(style::hit_bg(), 2.)]
            } else {
                Vec::new()
            };
            let mark = match tick.kind {
                TickKind::Tool => div().w(px(4.)).h(px(12.)).rounded(px(1.)).bg(if tick.error { style::turn_err() } else { style::tick() }),
                TickKind::Text => div().size(px(5.)).rounded_full().bg(style::muted()),
                TickKind::Thinking => div().size(px(5.)).rounded_full().border_1().border_color(style::tick_thinking()),
            };
            div()
                .id(element_id("tick", c, s))
                .w(px(if tick.kind == TickKind::Tool { 6. } else { 7. }))
                .h(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| this.hover_tick(c, s, *hovered, cx)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.detail.point_at(c, s);
                    cx.notify();
                }))
                .child(mark.when(!rings.is_empty(), |el| el.shadow(rings)))
        }))
    }

    fn hover_tick(&mut self, card: usize, step: usize, hovered: bool, cx: &mut Context<Self>) {
        let tick = Some((card, step));
        if hovered {
            self.detail.hover_tick = tick;
        } else if self.detail.hover_tick == tick {
            self.detail.hover_tick = None;
        } else {
            return;
        }
        cx.notify();
    }

    /// `Read 12 · Bash 11 · text 2`, or what the hovered tick's step is:
    /// `14 / 29  Bash  cargo test …  error · 46 lines`.
    fn render_stats(&self, c: usize, t: &TurnCard, cx: &mut Context<Self>) -> Div {
        let hovered = match self.detail.hover_tick {
            Some((card, step)) if card == c => t.steps.get(step).map(|view| hover_line(step, t.steps.len(), view)),
            _ => None,
        };
        div()
            .font_family(style::mono(cx))
            .text_size(px(10.))
            .line_height(px(14.))
            .text_color(style::muted())
            .truncate()
            .child(hovered.unwrap_or_else(|| StyledText::new(t.stats.clone())))
    }

    fn render_step(&self, c: usize, s: usize, t: &TurnCard, cx: &mut Context<Self>) -> Div {
        let Some(step) = t.steps.get(s) else { return div() };
        let body = match &step.body {
            StepBody::Text(text) => self.render_process_text(c, s, step, text, cx),
            _ => self.render_step_row(c, s, step, cx),
        };
        row_frame(false).child(card_slice(false, false).child(body.border_t_1().border_color(if s == 0 { style::step_border() } else { style::step_sep() })))
    }

    /// Process text between tool calls: shown like a reply, never collapsed to a row.
    fn render_process_text(&self, c: usize, s: usize, step: &StepView, text: &Folded, cx: &mut Context<Self>) -> Div {
        let open = self.detail.unfolded.contains(&(c, Part::Step(s)));
        let fg = style::muted_light();
        v_flex().child(
            h_flex()
                .items_start()
                .gap(px(8.))
                .pl(px(10.))
                .pr(px(12.))
                .py(px(5.))
                .when(self.detail.pointed == Some((c, s)), |el| el.bg(style::lit_bg()))
                .child(label_column(Some("ASST".into()), step.time.clone(), fg, fg, cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(px(11.5)).line_height(px(15.)).text_color(style::text_secondary()).child(self.marked(text, open, c, Part::Step(s), cx)))
                        .children(cut_note(text, open))
                        .children(self.fold_link(text, c, Part::Step(s), cx)),
                ),
        )
    }

    /// A tool call or a thinking: one row (time, ▶, name, argument, result), and its input /
    /// output (or the thinking's text) once open.
    fn render_step_row(&self, c: usize, s: usize, step: &StepView, cx: &mut Context<Self>) -> Div {
        let open = self.detail.open_steps.contains(&(c, s));
        let lit = self.detail.pointed == Some((c, s));
        let thinking = step.kind == TickKind::Thinking;
        let name = if thinking {
            div().flex_none().font_family(style::sans(cx)).italic().text_color(style::muted_light()).child(step.label.clone())
        } else {
            // the mockup's 56px fit Read / Bash / Edit; longer names widen their own row
            div().min_w(px(TOOL_NAME_W)).max_w(px(TOOL_NAME_MAX_W)).flex_none().truncate().font_weight(FontWeight::SEMIBOLD).text_color(style::tool_name()).child(step.label.clone())
        };
        let row = h_flex()
            .id(element_id("step", c, s))
            .h(px(STEP_ROW_H))
            .items_center()
            .gap(px(8.))
            .pl(px(10.))
            .pr(px(12.))
            .font_family(style::mono(cx))
            .text_size(px(11.))
            .cursor_default()
            .map(|el| if lit { el.bg(style::lit_bg()) } else { el.hover(|el| el.bg(style::row_hover())) })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.detail.toggle_step(c, s);
                cx.notify();
            }))
            .child(div().w(px(LABEL_W)).flex_none().text_size(px(10.)).text_color(style::muted_light()).child(step.time.clone()))
            .child(div().w(px(12.)).flex_none().flex().justify_center().text_size(px(8.)).text_color(style::muted()).child(if open { "▼" } else { "▶" }))
            .child(name)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(thinking, |el| el.font_family(style::sans(cx)))
                    .text_color(if thinking { style::muted_light() } else { style::text_secondary() })
                    .child(step.summary.clone()),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap(px(4.))
                    .whitespace_nowrap()
                    .text_size(px(10.))
                    .text_color(if step.error { style::error_text() } else { style::muted() })
                    .child(step.meta.clone())
                    .when(!step.hits_fmt.is_empty(), |el| el.child(hits_badge(step.hits_fmt.clone(), cx))),
            );
        v_flex().child(row).when(open, |el| el.child(self.render_step_box(c, s, step, lit, cx)))
    }

    /// Under an open step: `input` and `output` boxes, or the whole thinking.
    fn render_step_box(&self, c: usize, s: usize, step: &StepView, lit: bool, cx: &mut Context<Self>) -> Div {
        let (marks, current) = (self.marks(c, Part::Step(s)), self.is_current(c, Part::Step(s)));
        let code_box = |cx: &App| {
            v_flex()
                .font_family(style::mono(cx))
                .text_size(px(11.))
                .line_height(px(15.))
                .bg(style::white())
                .border_1()
                .border_color(style::hairline())
                .rounded(px(4.))
                .px(px(8.))
                .py(px(4.))
        };
        let caption = |text: &'static str| div().text_size(px(10.)).line_height(px(13.)).text_color(style::muted()).mb(px(2.)).child(text);
        let content = match &step.body {
            StepBody::Tool { input, output } => {
                let output_open = self.detail.unfolded.contains(&(c, Part::Step(s)));
                v_flex()
                    .gap(px(6.))
                    .when(!step.full_name.is_empty(), |el| {
                        el.child(div().font_family(style::mono(cx)).text_size(px(10.)).line_height(px(13.)).text_color(style::muted()).child(step.full_name.clone()))
                    })
                    .when(!input.is_empty(), |el| {
                        el.child(v_flex().child(caption("input")).child(code_box(cx).children(input.iter().map(|line| {
                            div()
                                .text_color(if line.kind == InputKind::Description { style::muted() } else { style::asst_text() })
                                .child(self.revealing(StyledText::new(line.text.clone()).with_highlights(line.hits.iter().map(|r| (r.clone(), marks))), line.hits.first().map(|hit| hit.start), current))
                                .when(!line.cut.is_empty(), |el| el.child(cut_line(line.cut.clone())))
                        }))))
                    })
                    .children(output.as_ref().filter(|out| !out.text.is_empty()).map(|out| {
                        v_flex()
                            .child(caption("output"))
                            .child(code_box(cx).text_color(if step.error { style::error_text() } else { style::asst_text() }).child(self.marked(out, output_open, c, Part::Step(s), cx)).children(cut_note(out, output_open)))
                            .when(out.fold.is_some(), |el| {
                                el.child(
                                    div()
                                        .id(element_id("output-fold", c, s))
                                        .mt(px(2.))
                                        .text_size(px(10.))
                                        .text_color(style::muted_light())
                                        .cursor_default()
                                        .hover(|el| el.text_decoration_1())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.detail.toggle_fold(c, Part::Step(s));
                                            cx.notify();
                                        }))
                                        .child(if output_open { "Show less".into() } else { out.more.clone() }),
                                )
                            })
                    }))
            }
            StepBody::Thinking(text) | StepBody::Text(text) => {
                v_flex()
                    .child(div().text_size(px(11.)).line_height(px(15.)).italic().text_color(style::head_fg()).child(self.marked(text, true, c, Part::Step(s), cx)))
                    .children(cut_note(text, true))
            }
        };
        v_flex()
            .pl(px(BODY_X))
            .pr(px(12.))
            .pt(px(6.))
            .pb(px(8.))
            .bg(if lit { style::lit_box_bg() } else { style::expanded_bg() })
            .border_t_1()
            .border_color(style::expanded_border())
            .child(content)
    }

    /// The reply, the interruption or the API error, and the card's bottom edge.
    fn render_tail(&self, c: usize, t: &TurnCard, cx: &mut Context<Self>) -> Div {
        let open = self.detail.unfolded.contains(&(c, Part::Reply));
        let body = match &t.tail {
            Tail::Empty => None,
            Tail::Reply { time, text } => Some(
                h_flex()
                    .items_start()
                    .gap(px(8.))
                    .pl(px(10.))
                    .pr(px(12.))
                    .pt(px(6.))
                    .pb(px(8.))
                    .child(label_column(Some("ASST".into()), time.clone(), style::text_secondary(), style::muted(), cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(12.)).line_height(px(16.)).text_color(style::asst_text()).child(self.marked(text, open, c, Part::Reply, cx)))
                            .children(cut_note(text, open))
                            .children(self.fold_link(text, c, Part::Reply, cx)),
                    ),
            ),
            Tail::Interrupted { time } => Some(
                h_flex()
                    .items_start()
                    .gap(px(8.))
                    .pl(px(10.))
                    .pr(px(12.))
                    .pt(px(6.))
                    .pb(px(7.))
                    .child(label_column(None, time.clone(), style::muted(), style::muted(), cx))
                    .child(div().text_size(px(11.)).line_height(px(14.)).italic().text_color(style::muted()).child("[Request interrupted by user]")),
            ),
            Tail::ApiError { time, text } => Some(
                h_flex()
                    .items_start()
                    .gap(px(8.))
                    .pl(px(10.))
                    .pr(px(12.))
                    .pt(px(6.))
                    .pb(px(7.))
                    .child(label_column(None, time.clone(), style::muted(), style::muted(), cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_family(style::mono(cx))
                                    .text_size(px(10.5))
                                    .line_height(px(14.))
                                    .text_color(style::api_error_fg())
                                    .bg(style::api_error_bg())
                                    .border_1()
                                    .border_color(style::api_error_border())
                                    .rounded(px(4.))
                                    .px(px(8.))
                                    .py(px(4.))
                                    .child(folded_text(text, open, style::hit_highlight())),
                            )
                            .children(self.fold_link(text, c, Part::Reply, cx)),
                    ),
            ),
        };
        let edge = card_slice(false, true);
        row_frame(false).child(match body {
            Some(body) => edge.child(body.border_t_1().border_color(style::step_border())),
            None => edge.h(px(6.)),
        })
    }

    /// `──── context compacted · 22:05  summary · 9.8k chars ▸ ────`, and the summary once open.
    fn render_compact(&self, c: usize, k: &CompactCard, cx: &mut Context<Self>) -> Div {
        let open = self.detail.unfolded.contains(&(c, Part::Summary));
        let rule = || div().flex_1().h(px(1.)).bg(style::compact_rule());
        v_flex()
            .w_full()
            .px(px(CARD_X))
            .pt(px(12.))
            .pb(px(4.))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .font_family(style::mono(cx))
                    .text_size(px(10.5))
                    .line_height(px(14.))
                    .text_color(style::muted())
                    .child(rule())
                    .child(div().flex_none().child(k.label.clone()))
                    .when(!k.summary_label.is_empty(), |el| {
                        el.child(
                            h_flex()
                                .id(element_id("summary", c, 0))
                                .flex_none()
                                .whitespace_nowrap()
                                .font_family(style::sans(cx))
                                .text_color(style::accent())
                                .cursor_default()
                                .hover(|el| el.text_decoration_1())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.detail.toggle_fold(c, Part::Summary);
                                    cx.notify();
                                }))
                                .child(k.summary_label.clone())
                                .child(if open { " ▾" } else { " ▸" }),
                        )
                    })
                    .when(k.hits > 0, |el| el.child(hits_badge(k.hits.to_string().into(), cx)))
                    .child(rule()),
            )
            .when(open, |el| {
                el.children(k.summary.as_ref().map(|summary| {
                    div()
                        .mt(px(6.))
                        .px(px(12.))
                        .py(px(8.))
                        .bg(style::white())
                        .border_1()
                        .border_color(style::card_border())
                        .rounded(px(CARD_RADIUS))
                        .text_size(px(11.))
                        .line_height(px(15.))
                        .text_color(style::text_secondary())
                        .child(self.marked(summary, true, c, Part::Summary, cx))
                        .children(cut_note(summary, true))
                }))
            })
    }
}

/// Hits this far below the top of their text are scrolled to; nearer ones are on screen once
/// their row is at the top.
const REVEAL_BELOW: f32 = 240.;
/// Lines of context left above a revealed hit.
const REVEAL_LEAD_LINES: f32 = 3.;
/// What the detail list leaves of the window's height (toolbar, detail header, bars).
const LIST_CHROME_H: f32 = 300.;

/// Zero-size marker after a text holding the current hit: on the first prepaint after a park
/// (the text before it is laid out by then) it asks the list to scroll the hit near the top.
/// `top` is the layout of the text's first piece (where the text starts), `layout` that of the
/// piece with the hit, `at` the hit's place in it; a text shown whole is one piece.
fn reveal_marker(pending: std::rc::Rc<std::cell::Cell<bool>>, top: TextLayout, layout: TextLayout, at: usize) -> impl IntoElement {
    canvas(
        move |_, window, _| {
            if !pending.replace(false) {
                return;
            }
            let Some(hit) = layout.position_for_index(at) else { return };
            let line = layout.line_height();
            if hit.y - top.bounds().top() < px(REVEAL_BELOW) {
                return;
            }
            // as tall as the list, so the list puts the hit near its top whichever way it scrolls
            let height = (window.viewport_size().height - px(LIST_CHROME_H)).max(line * 4.);
            window.request_autoscroll(Bounds::new(point(hit.x, hit.y - line * REVEAL_LEAD_LINES), size(px(1.), height)));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size(px(0.))
}

/// The stats line while a tick is hovered: position, name, argument, result.
fn hover_line(step: usize, steps: usize, view: &StepView) -> StyledText {
    let mut text = format!("{} / {steps}", step + 1);
    let mut marks = vec![(0..text.len(), HighlightStyle { color: Some(style::accent()), ..Default::default() })];
    text.push_str("  ");
    let name = text.len()..text.len() + view.label.len();
    text.push_str(&view.label);
    if view.kind == TickKind::Tool {
        marks.push((name, HighlightStyle { color: Some(style::tool_name()), font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() }));
    }
    if !view.summary.is_empty() {
        text.push_str("  ");
        text.push_str(&view.summary);
    }
    if !view.meta.is_empty() {
        text.push_str("  ");
        let meta = text.len()..text.len() + view.meta.len();
        text.push_str(&view.meta);
        if view.error {
            marks.push((meta, HighlightStyle { color: Some(style::error_text()), ..Default::default() }));
        }
    }
    StyledText::new(text).with_highlights(marks)
}
