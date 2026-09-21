//! Detail panel: header + Metadata / Conversation tabs.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::*;

use crate::model::{CONVERSATION_CAP, Conversation, Kind, Message, MessageKind, SessionRow};
use crate::style::{self, FluentBuilder};
use crate::AppRoot;

pub const DETAIL_W: f32 = 580.;
const LABEL_COL_W: f32 = 58.;
const COLLAPSED_ROW_H: f32 = 22.;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Metadata,
    Conversation,
}

/// Which message kinds the conversation view shows (User / Assistant / Tools / Thinking).
#[derive(Clone, Copy)]
pub struct Chips([bool; 4]);

impl Chips {
    const LABELS: [&'static str; 4] = ["User", "Assistant", "Tools", "Thinking"];

    fn ix(kind: MessageKind) -> usize {
        match kind {
            MessageKind::User => 0,
            MessageKind::Assistant => 1,
            MessageKind::Tool => 2,
            MessageKind::Thinking => 3,
        }
    }

    fn shows(&self, kind: MessageKind) -> bool {
        self.0[Self::ix(kind)]
    }
}

pub struct DetailState {
    pub tab: DetailTab,
    pub chips: Chips,
    expanded: HashMap<usize, bool>,
    force_all: Option<bool>,
    pub conversation: Conversation,
    /// Indexed messages of the session, including the ones beyond `CONVERSATION_CAP`.
    total_messages: usize,
}

impl Default for DetailState {
    fn default() -> Self {
        Self {
            tab: DetailTab::Conversation,
            chips: Chips([true, true, true, true]),
            expanded: HashMap::new(),
            force_all: None,
            conversation: Vec::new(),
            total_messages: 0,
        }
    }
}

impl DetailState {
    pub fn set_conversation(&mut self, conversation: Conversation, total_messages: usize) {
        self.conversation = conversation;
        self.total_messages = total_messages;
        self.expanded.clear();
        self.force_all = None;
    }

    fn is_open(&self, ix: usize) -> bool {
        self.expanded.get(&ix).copied().unwrap_or(self.force_all.unwrap_or(false))
    }

    fn toggle(&mut self, ix: usize) {
        let open = self.is_open(ix);
        self.expanded.insert(ix, !open);
    }

    fn visible(&self) -> impl Iterator<Item = (usize, &Arc<Message>)> {
        self.conversation.iter().enumerate().filter(|(_, m)| self.chips.shows(m.kind))
    }

    fn any_closed(&self) -> bool {
        self.visible().any(|(ix, m)| matches!(m.kind, MessageKind::Tool | MessageKind::Thinking) && !self.is_open(ix))
    }

    fn counts(&self) -> [usize; 4] {
        let mut counts = [0; 4];
        for m in &self.conversation {
            counts[Chips::ix(m.kind)] += 1;
        }
        counts
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
        let dot = || div().text_color(rgb(0xc4c4c8)).child("·");
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
            .child(div().truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).line_height(px(18.)).child(row.title.clone()))
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
                        meta_pair("Cost", mono_value(row.cost_fmt.clone(), cx).into_any_element()),
                    ))
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

    fn render_chip_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let counts = self.detail.counts();
        let any_closed = self.detail.any_closed();
        h_flex()
            .flex_none()
            .h(px(28.))
            .items_center()
            .gap(px(4.))
            .px(px(10.))
            .bg(style::head_bg())
            .border_b_1()
            .border_color(style::hairline())
            .children(Chips::LABELS.iter().enumerate().map(|(ix, label)| {
                let on = self.detail.chips.0[ix];
                h_flex()
                    .id(("chip", ix))
                    .h(px(18.))
                    .items_center()
                    .gap(px(4.))
                    .px(px(7.))
                    .rounded_full()
                    .border_1()
                    .border_color(if on { style::chip_border() } else { cx.theme().transparent })
                    .bg(if on { style::white() } else { cx.theme().transparent })
                    .text_size(px(11.))
                    .text_color(if on { style::text() } else { style::muted_light() })
                    .cursor_default()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.detail.chips.0[ix] = !this.detail.chips.0[ix];
                        cx.notify();
                    }))
                    .child(*label)
                    .child(div().font_family(mono(cx)).text_size(px(10.)).opacity(0.7).child(counts[ix].to_string()))
            }))
            .child(div().flex_1())
            .child(
                div()
                    .id("toggle-all")
                    .text_size(px(11.))
                    .text_color(style::accent())
                    .cursor_default()
                    .hover(|el| el.text_decoration_1())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.detail.expanded.clear();
                        this.detail.force_all = Some(any_closed);
                        cx.notify();
                    }))
                    .child(if any_closed { "Expand all" } else { "Collapse all" }),
            )
    }

    /// Footer under the messages: what this view cannot show yet.
    fn conversation_note(&self, row: &SessionRow) -> SharedString {
        if row.kind == Kind::History {
            return "The transcript was cleaned up by Claude Code; only the prompts kept in history.jsonl remain (see Metadata).".into();
        }
        if self.detail.total_messages > CONVERSATION_CAP {
            return format!("Showing the first {CONVERSATION_CAP} of {} indexed messages. Tool calls and thinking are not shown yet.", self.detail.total_messages).into();
        }
        "Indexed user / assistant text only. Tool calls and thinking are not shown yet.".into()
    }

    fn render_conversation(&self, row: &SessionRow, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let note = self.conversation_note(row);
        let mut items: Vec<AnyElement> = Vec::new();
        let mut turn = 0;
        for (ix, m) in self.detail.visible() {
            let el = match m.kind {
                MessageKind::User => {
                    turn += 1;
                    self.render_user(m, turn, !items.is_empty(), cx).into_any_element()
                }
                MessageKind::Assistant => self.render_assistant(m, cx).into_any_element(),
                MessageKind::Thinking => self.render_thinking(ix, m, cx).into_any_element(),
                MessageKind::Tool => self.render_tool(ix, m, cx).into_any_element(),
            };
            items.push(el);
        }
        v_flex()
            .flex_1()
            .min_h_0()
            .child(self.render_chip_bar(cx))
            .child(div().id("conversation").flex_1().min_h_0().overflow_y_scrollbar().child(
                v_flex().children(items).child(div().px(px(12.)).py(px(10.)).text_size(px(11.)).text_color(style::muted_light()).child(note)),
            ))
    }

    fn render_user(&self, m: &Arc<Message>, turn: usize, gap_top: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        h_flex()
            .items_start()
            .gap(px(8.))
            .pl(px(10.))
            .pr(px(12.))
            .py(px(7.))
            .when(gap_top, |el| el.mt(px(4.)))
            .bg(style::user_bg())
            .border_t_1()
            .border_b_1()
            .border_color(style::user_border())
            .child(
                v_flex()
                    .w(px(LABEL_COL_W))
                    .flex_none()
                    .font_family(mono(cx))
                    .text_size(px(10.))
                    .line_height(px(14.))
                    .child(div().font_weight(FontWeight::BOLD).text_color(style::accent()).child(format!("USER #{turn}")))
                    .child(div().text_color(style::user_time()).child(m.time.clone())),
            )
            .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(16.)).font_weight(FontWeight::MEDIUM).child(m.text.clone()))
    }

    fn render_assistant(&self, m: &Arc<Message>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        h_flex()
            .items_start()
            .gap(px(8.))
            .pl(px(10.))
            .pr(px(12.))
            .py(px(6.))
            .border_b_1()
            .border_color(rgb(0xf2f2f4))
            .child(
                v_flex()
                    .w(px(LABEL_COL_W))
                    .flex_none()
                    .font_family(mono(cx))
                    .text_size(px(10.))
                    .line_height(px(14.))
                    .text_color(style::muted())
                    .child(div().font_weight(FontWeight::BOLD).text_color(style::text_secondary()).child("ASST"))
                    .child(m.time.clone()),
            )
            .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(16.)).text_color(style::asst_text()).child(m.text.clone()))
    }

    fn collapsed_row(&self, ix: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        h_flex()
            .id(("msg", ix))
            .h(px(COLLAPSED_ROW_H))
            .items_center()
            .gap(px(8.))
            .pl(px(10.))
            .pr(px(12.))
            .border_b_1()
            .border_color(rgb(0xf2f2f4))
            .cursor_default()
            .hover(|el| el.bg(style::row_hover()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.detail.toggle(ix);
                cx.notify();
            }))
    }

    fn time_and_chevron(&self, m: &Arc<Message>, open: bool, color: Hsla, cx: &App) -> [AnyElement; 2] {
        [
            div().w(px(LABEL_COL_W)).flex_none().font_family(mono(cx)).text_size(px(10.)).text_color(color).child(m.time.clone()).into_any_element(),
            div().w(px(12.)).flex_none().text_size(px(9.)).text_color(style::muted()).child(if open { "▼" } else { "▶" }).into_any_element(),
        ]
    }

    fn expanded_box(&self) -> Div {
        v_flex().pl(px(76.)).pr(px(12.)).pt(px(6.)).pb(px(8.)).border_b_1().border_color(rgb(0xf2f2f4)).bg(style::expanded_bg())
    }

    fn render_thinking(&self, ix: usize, m: &Arc<Message>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let open = self.detail.is_open(ix);
        v_flex()
            .child(
                self.collapsed_row(ix, cx)
                    .text_color(style::muted_light())
                    .children(self.time_and_chevron(m, open, style::muted_light(), cx))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(4.))
                            .text_size(px(11.))
                            .child(div().flex_none().italic().child("thinking"))
                            .when(!open, |el| el.child(div().truncate().opacity(0.8).child(m.preview.clone()))),
                    )
                    .child(div().flex_none().font_family(mono(cx)).text_size(px(10.)).child(m.meta.clone())),
            )
            .when(open, |el| {
                el.child(
                    self.expanded_box().child(
                        div().text_size(px(11.)).line_height(px(15.)).italic().text_color(style::head_fg()).child(m.text.clone()),
                    ),
                )
            })
    }

    fn render_tool(&self, ix: usize, m: &Arc<Message>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let open = self.detail.is_open(ix);
        let mono_font = mono(cx);
        let code_box = move |text: SharedString, color: Hsla| {
            div()
                .font_family(mono_font.clone())
                .text_size(px(11.))
                .line_height(px(15.))
                .text_color(color)
                .bg(style::white())
                .border_1()
                .border_color(style::hairline())
                .rounded(px(4.))
                .px(px(8.))
                .py(px(4.))
                .child(text)
        };
        v_flex()
            .child(
                self.collapsed_row(ix, cx)
                    .font_family(mono(cx))
                    .text_size(px(11.))
                    .children(self.time_and_chevron(m, open, style::muted_light(), cx))
                    .child(div().w(px(56.)).flex_none().font_weight(FontWeight::SEMIBOLD).text_color(style::tool_name()).child(m.name.clone()))
                    .child(div().flex_1().min_w_0().truncate().text_color(style::text_secondary()).child(m.arg.clone()))
                    .child(div().flex_none().text_size(px(10.)).text_color(style::muted()).whitespace_nowrap().child(m.result.clone())),
            )
            .when(open, |el| {
                el.child(
                    self.expanded_box()
                        .gap(px(6.))
                        .child(
                            v_flex()
                                .child(div().text_size(px(10.)).text_color(style::muted()).mb(px(2.)).child("input"))
                                .child(code_box(m.input.clone(), style::asst_text())),
                        )
                        .child(
                            v_flex()
                                .child(div().text_size(px(10.)).text_color(style::muted()).mb(px(2.)).child("output"))
                                .child(code_box(m.output.clone(), style::text_secondary()))
                                .when(!m.more.is_empty(), |el| {
                                    el.child(div().text_size(px(10.)).text_color(style::muted_light()).mt(px(2.)).child(m.more.clone()))
                                }),
                        ),
                )
            })
    }
}
