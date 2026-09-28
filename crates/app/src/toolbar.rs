//! Toolbar: `Label value ▾` dropdowns (Project / Scope / Updated / Kind / Preview), the search box
//! with its mode toggles, and the search life cycle (debounce → `Searcher` → hits into the table).

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, h_flex};
use gpui_kit::*;

use crate::model::{DisplayMode, Filter, Kind, RangePreset, ScopeSet};
use crate::search::{LABELS, MAX_RESULT_SESSIONS, SearchEvent, SearchMode};
use crate::style::{self, FluentBuilder};
use crate::AppRoot;

pub const TOOLBAR_H: f32 = 36.;
const CONTROL_H: f32 = 22.;
const MENU_ITEM_H: f32 = 24.;
/// Typing pause before a search starts.
pub(crate) const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);

/// `Label value ▾` trigger button styled like the mockup.
fn trigger(id: &'static str, label: &'static str, value: SharedString, cx: &App) -> Button {
    let variant = ButtonCustomVariant::new(cx)
        .color(style::white())
        .foreground(style::text())
        .hover(style::control_hover())
        .active(style::control_hover());
    Button::new(id)
        .custom(variant)
        .small()
        .h(px(CONTROL_H))
        .pl(px(8.))
        .pr(px(6.))
        .border_1()
        .border_color(style::control_border())
        .rounded(px(5.))
        .dropdown_caret(true)
        .child(
            h_flex()
                .gap(px(4.))
                .text_size(px(12.))
                .child(div().text_color(style::label()).child(label))
                .child(div().font_weight(FontWeight::MEDIUM).child(value)),
        )
}

/// Square on / off button next to the search box (`.*`, `Aa`).
fn toggle(id: &'static str, glyph: &'static str, on: bool, tooltip: &'static str, cx: &App) -> Button {
    let (bg, fg) = if on { (style::toggle_on_bg(), style::toggle_on_fg()) } else { (style::white(), style::label()) };
    let hover = if on { bg } else { style::control_hover() };
    Button::new(id)
        .custom(ButtonCustomVariant::new(cx).color(bg).foreground(fg).hover(hover).active(hover))
        .small()
        .h(px(CONTROL_H))
        .w(px(26.))
        .px(px(0.))
        .border_1()
        .border_color(if on { style::toggle_on_fg().opacity(0.35) } else { style::control_border() })
        .rounded(px(5.))
        .tooltip(tooltip)
        .child(div().font_family(style::mono(cx)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).child(glyph))
}

/// One row inside a dropdown: `name` on the left, an optional right-aligned `hint`.
fn menu_row(name: SharedString, hint: SharedString, hint_mono: bool, cx: &App) -> Div {
    h_flex()
        .h(px(MENU_ITEM_H))
        .w_full()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .child(div().flex_1().truncate().child(name))
        .when(!hint.is_empty(), |el| {
            el.child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .opacity(0.6)
                    .when(hint_mono, |el| el.font_family(style::mono(cx)))
                    .child(hint),
            )
        })
}

fn menu_item(
    name: SharedString,
    hint: SharedString,
    hint_mono: bool,
    checked: bool,
    on_click: impl Fn(&mut App) + 'static,
) -> PopupMenuItem {
    PopupMenuItem::element(move |_, cx| menu_row(name.clone(), hint.clone(), hint_mono, cx))
        .checked(checked)
        .on_click(move |_, _, cx| on_click(cx))
}

impl AppRoot {
    fn current_filter(&self, cx: &App) -> Filter {
        self.table.read(cx).delegate().filter().clone()
    }

    /// `project` is a `project_key`.
    pub fn set_project(&mut self, project: Option<SharedString>, cx: &mut Context<Self>) {
        let mut filter = self.current_filter(cx);
        filter.project = project;
        self.apply_filter(filter, cx);
    }

    /// Hidden projects never show, whatever else is picked; the Project dropdown lists them apart.
    pub fn hide_project(&mut self, key: SharedString, cx: &mut Context<Self>) {
        let mut filter = self.current_filter(cx);
        filter.hide_project(key);
        self.apply_filter(filter, cx);
    }

    pub fn show_project(&mut self, key: &str, cx: &mut Context<Self>) {
        let mut filter = self.current_filter(cx);
        filter.show_project(key);
        self.apply_filter(filter, cx);
    }

    pub fn hide_selected_project(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.selected.as_ref().map(|row| row.project_key.clone()) {
            self.hide_project(key, cx);
        }
    }

    pub fn set_range(&mut self, range: RangePreset, cx: &mut Context<Self>) {
        self.range = range;
        let mut filter = self.current_filter(cx);
        filter.since_ms = range.since_ms();
        self.apply_filter(filter, cx);
    }

    pub fn toggle_kind(&mut self, kind: Kind, cx: &mut Context<Self>) {
        let mut filter = self.current_filter(cx);
        filter.kinds.toggle(kind);
        self.apply_filter(filter, cx);
    }

    pub fn toggle_empty_sessions(&mut self, cx: &mut Context<Self>) {
        let mut filter = self.current_filter(cx);
        filter.show_empty = !filter.show_empty;
        self.apply_filter(filter, cx);
    }

    fn apply_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        let (changed, ms, shown) = self.table.update(cx, |table, cx| {
            let changed = table.delegate_mut().set_filter(filter);
            if changed {
                table.scroll_to_row(0, cx);
                cx.notify();
            }
            (changed, table.delegate().last_filter_ms, table.delegate().ix.len())
        });
        if changed {
            println!("filter + sort: {shown} rows in {ms}ms");
            self.status = format!("filtered {shown} sessions in {ms}ms").into();
            self.after_rows_changed(cx);
            self.persist(cx);
            // the search looks into the filtered sessions only, so it has to look again
            if self.searching() {
                self.run_search(cx);
            }
        }
        cx.notify();
    }

    /// A search is showing its results or on its way.
    pub fn searching(&self) -> bool {
        !self.query.is_empty()
    }

    /// The search box changed: wait for a pause in typing, then search.
    pub fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self._debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            this.update(cx, |this, cx| this.run_search(cx)).ok();
        }));
    }

    /// Search for what the box holds now, in the sessions the filters let through. The table
    /// keeps the previous results until the first ones of this search arrive.
    pub fn run_search(&mut self, cx: &mut Context<Self>) {
        self.query = self.search.read(cx).value().trim().to_string().into();
        self.matcher = self.search_opts.matcher(&self.query);
        self.detail.set_matcher(self.matcher.as_ref());
        self.search_summary = None;
        self.search_error = None;
        if self.query.is_empty() {
            self.end_search(cx);
            return;
        }
        let targets = self.table.read(cx).delegate().search_targets();
        self.search_seq = self.searcher.search(self.query.to_string(), self.search_opts.clone(), targets);
        self.search_pending = true;
        cx.notify();
    }

    fn end_search(&mut self, cx: &mut Context<Self>) {
        self.search_seq = self.searcher.cancel();
        self.search_pending = false;
        self.show_search_results(false, cx);
        self.after_rows_changed(cx);
        cx.notify();
    }

    /// Switch the table into (a fresh) search mode or out of it.
    fn show_search_results(&mut self, active: bool, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            if !active && !table.delegate().search_active() {
                return;
            }
            table.delegate_mut().set_search(active);
            table.refresh(cx);
            table.scroll_to_row(0, cx);
        });
    }

    /// Apply a batch of search events; anything of a superseded search is dropped.
    pub fn on_search_events(&mut self, batch: Vec<SearchEvent>, cx: &mut Context<Self>) {
        let seq_of = |ev: &SearchEvent| match ev {
            SearchEvent::Hits { seq, .. } | SearchEvent::Done { seq, .. } | SearchEvent::Failed { seq, .. } => *seq,
        };
        let current: Vec<SearchEvent> = batch.into_iter().filter(|ev| seq_of(ev) == self.search_seq).collect();
        if current.is_empty() {
            return;
        }
        if std::mem::take(&mut self.search_pending) {
            self.show_search_results(true, cx);
        }
        for ev in current {
            match ev {
                SearchEvent::Hits { hits, .. } => self.table.update(cx, |table, _| table.delegate_mut().add_hits(hits)),
                SearchEvent::Done { summary, .. } => {
                    println!("search {:?} ({:?}): {summary:?}", self.query, self.search_opts.mode);
                    self.search_summary = Some(summary);
                }
                SearchEvent::Failed { message, .. } => {
                    eprintln!("search: {message}");
                    self.search_error = Some(message.into());
                }
            }
        }
        self.after_rows_changed(cx);
        cx.notify();
    }

    /// Status bar text of the search: (what it found, how long it took).
    pub fn search_status(&self) -> Option<(SharedString, SharedString)> {
        if !self.searching() {
            return None;
        }
        if let Some(message) = &self.search_error {
            return Some((format!("search failed: {message}").into(), "".into()));
        }
        let scope = self.search_opts.scope;
        if self.search_opts.mode == SearchMode::Text && !scope.user && !scope.assistant {
            return Some(("nothing to search: the Tools scope needs regex mode (.*)".into(), "".into()));
        }
        let Some(summary) = &self.search_summary else { return Some(("searching…".into(), "".into())) };
        let mut found = format!("{} sessions · {} matches", summary.total_sessions.unwrap_or(summary.sessions), summary.matches);
        if summary.truncated {
            let more = if summary.total_sessions.is_some() { "" } else { ", more match" };
            found.push_str(&format!(" · newest {MAX_RESULT_SESSIONS} shown{more}"));
        }
        let ms = summary.elapsed.as_secs_f64() * 1000.0;
        let took = if ms < 10.0 { format!("search {ms:.1}ms") } else { format!("search {ms:.0}ms") };
        Some((found.into(), took.into()))
    }

    /// Scope / mode / case / labels changed: remember it and search again.
    pub(crate) fn search_options_changed(&mut self, cx: &mut Context<Self>) {
        self.persist(cx);
        if self.searching() {
            self.run_search(cx);
        }
        cx.notify();
    }

    pub fn toggle_regex(&mut self, cx: &mut Context<Self>) {
        self.search_opts.mode = match self.search_opts.mode {
            SearchMode::Text => SearchMode::Regex,
            SearchMode::Regex => SearchMode::Text,
        };
        self.search_options_changed(cx);
    }

    pub fn toggle_case(&mut self, cx: &mut Context<Self>) {
        self.search_opts.case_sensitive = !self.search_opts.case_sensitive;
        self.search_options_changed(cx);
    }

    pub fn set_mode(&mut self, mode: DisplayMode, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().mode = mode;
            cx.notify();
        });
        self.persist(cx);
        cx.notify();
    }

    fn project_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let current = self.current_filter(cx).project;
        let total: usize = self.projects.iter().map(|p| p.count).sum();
        let projects = self.projects.clone();
        let hidden = self.hidden_projects.clone();
        let picked = current.as_ref().and_then(|key| projects.iter().find(|p| p.key == *key)).cloned();
        move |menu, _, _| {
            let this_all = this.clone();
            let mut menu = menu.min_w(px(248.)).max_h(px(420.)).scrollable(true).item(menu_item(
                "All projects".into(),
                total.to_string().into(),
                true,
                current.is_none(),
                move |cx| {
                    this_all.update(cx, |app, cx| app.set_project(None, cx)).ok();
                },
            ));
            // extra rows only show up once a project is picked or hidden, so the default list keeps its layout
            if let Some(picked) = &picked {
                let this = this.clone();
                let key = picked.key.clone();
                menu = menu.item(menu_item(format!("Hide “{}”", picked.name).into(), "".into(), false, false, move |cx| {
                    let key = key.clone();
                    this.update(cx, |app, cx| app.hide_project(key, cx)).ok();
                }));
            }
            if !hidden.is_empty() {
                menu = menu.separator().label("Hidden · click to show again");
            }
            for project in &hidden {
                let this = this.clone();
                let key = project.key.clone();
                menu = menu.item(menu_item(project.name.clone(), project.count.to_string().into(), true, false, move |cx| {
                    this.update(cx, |app, cx| app.show_project(&key, cx)).ok();
                }));
            }
            if picked.is_some() || !hidden.is_empty() {
                menu = menu.separator();
            }
            for project in &projects {
                let this = this.clone();
                let pick = project.key.clone();
                menu = menu.item(menu_item(
                    project.name.clone(),
                    project.count.to_string().into(),
                    true,
                    current.as_ref() == Some(&project.key),
                    move |cx| {
                        let pick = pick.clone();
                        this.update(cx, |app, cx| app.set_project(Some(pick), cx)).ok();
                    },
                ));
            }
            menu
        }
    }

    fn scope_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let scope = self.search_opts.scope;
        move |menu, _, _| {
            let mut menu = menu.min_w(px(200.));
            for (ix, (label, hint)) in ScopeSet::ITEMS.iter().enumerate() {
                let this = this.clone();
                menu = menu.item(menu_item((*label).into(), (*hint).into(), false, scope.get(ix), move |cx| {
                    this.update(cx, |app, cx| {
                        app.search_opts.scope.toggle(ix);
                        app.search_options_changed(cx);
                    })
                    .ok();
                }));
            }
            menu.separator().label("Applies to search. Tools = tool input/output, regex only.")
        }
    }

    fn label_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let picked = self.search_opts.labels.clone();
        move |menu, _, _| {
            let mut menu = menu.min_w(px(200.));
            for (label, hint) in LABELS {
                let this = this.clone();
                menu = menu.item(menu_item(label.into(), hint.into(), false, picked.iter().any(|l| l == label), move |cx| {
                    this.update(cx, |app, cx| {
                        app.search_opts.toggle_label(label);
                        app.search_options_changed(cx);
                    })
                    .ok();
                }));
            }
            menu.separator().label("Only matches inside these pieces. Regex mode.")
        }
    }

    fn range_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let current = self.range;
        move |menu, _, _| {
            let mut menu = menu.min_w(px(180.));
            for preset in RangePreset::ALL {
                let this = this.clone();
                menu = menu.item(menu_item(preset.label().into(), "".into(), false, preset == current, move |cx| {
                    this.update(cx, |app, cx| app.set_range(preset, cx)).ok();
                }));
            }
            menu
        }
    }

    fn kind_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let kinds = self.current_filter(cx).kinds;
        let counts = self.kind_counts;
        move |menu, _, _| {
            let mut menu = menu.min_w(px(160.));
            for kind in Kind::ALL {
                let this = this.clone();
                let count = counts[kind as usize].to_string();
                menu = menu.item(menu_item(kind.short().into(), count.into(), true, kinds.contains(kind), move |cx| {
                    this.update(cx, |app, cx| app.toggle_kind(kind, cx)).ok();
                }));
            }
            menu
        }
    }

    fn preview_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let current = self.table.read(cx).delegate().mode;
        move |menu, _, _| {
            let mut menu = menu.min_w(px(200.));
            for mode in DisplayMode::ALL {
                let this = this.clone();
                menu = menu.item(menu_item(mode.label().into(), mode.shortcut().into(), true, mode == current, move |cx| {
                    this.update(cx, |app, cx| app.set_mode(mode, cx)).ok();
                }));
            }
            menu
        }
    }

    pub fn render_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let filter = self.current_filter(cx);
        let project_label: SharedString = match (&filter.project, filter.hidden_projects.len()) {
            (Some(key), _) => self.projects.iter().find(|p| p.key == *key).map_or_else(|| key.clone(), |p| p.name.clone()),
            (None, 0) => "All".into(),
            (None, hidden) => format!("All · {hidden} hidden").into(),
        };
        let mode = self.table.read(cx).delegate().mode;
        let regex = self.search_opts.mode == SearchMode::Regex;
        let this = cx.entity().downgrade();
        let on_regex = this.clone();

        h_flex()
            .w_full()
            .h(px(TOOLBAR_H))
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .bg(style::toolbar_bg())
            .border_b_1()
            .border_color(style::border_soft())
            .child(trigger("project", "Project", project_label, cx).dropdown_menu(self.project_menu(cx)))
            .child(trigger("scope", "Scope", self.search_opts.scope.label().into(), cx).dropdown_menu(self.scope_menu(cx)))
            .child(trigger("range", "Updated", self.range.label().into(), cx).dropdown_menu(self.range_menu(cx)))
            .child(trigger("kind", "Kind", filter.kinds.label().into(), cx).dropdown_menu(self.kind_menu(cx)))
            .child(
                div().ml(px(4.)).w(px(220.)).child(
                    Input::new(&self.search)
                        .small()
                        .h(px(CONTROL_H))
                        .prefix(Icon::new(IconName::Search).xsmall().text_color(style::muted_light()))
                        .suffix(
                            div()
                                .font_family(style::mono(cx))
                                .text_size(px(10.))
                                .text_color(cx.theme().muted_foreground.opacity(0.8))
                                .child("⌘F"),
                        ),
                ),
            )
            .child(toggle("regex", ".*", regex, "Regex search over transcripts (⌥⌘R)", cx).on_click(move |_, _, cx| {
                on_regex.update(cx, |app, cx| app.toggle_regex(cx)).ok();
            }))
            .when(regex, |el| {
                el.child(toggle("case", "Aa", self.search_opts.case_sensitive, "Match case (⌥⌘C)", cx).on_click(move |_, _, cx| {
                    this.update(cx, |app, cx| app.toggle_case(cx)).ok();
                }))
                .child(trigger("labels", "Labels", self.search_opts.labels_label().into(), cx).dropdown_menu(self.label_menu(cx)))
            })
            .child(div().flex_1())
            .child(
                trigger("preview", "Preview", mode.label().into(), cx)
                    .dropdown_menu_with_anchor(Anchor::TopRight, self.preview_menu(cx)),
            )
    }
}
