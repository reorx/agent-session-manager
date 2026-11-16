//! Toolbar: the `Sessions | Usage` switch, then for the session table `Label value ▾` dropdowns
//! (Project / Scope / Updated / Kind / Preview), the search box with its mode toggles, and the
//! search life cycle (debounce → `Searcher` → hits into the table); for the Usage view its ranges,
//! the three filters it shares with the table, and when its data is from.

use std::time::Duration;

use chrono::{Datelike, Days, Months, NaiveDate};
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::date_picker::{DatePicker, DateRangePreset};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Sizable, h_flex};
use gpui_kit::*;

use crate::model::{DisplayMode, Filter, Kind, MainView, ModelEntry, RangePreset, ScopeSet};
use crate::usage_view::{RANGE_BUTTONS, fmt_day_in};
use agent_session_core::pricing::{display_name, normalize};
use agent_session_core::usage::UsageRange;
use crate::search::{LABELS, MAX_RESULT_SESSIONS, SearchEvent, SearchMode};
use crate::style::{self, FluentBuilder};
use crate::AppRoot;

pub const TOOLBAR_H: f32 = 36.;
const CONTROL_H: f32 = 22.;
const MENU_ITEM_H: f32 = 24.;
const DATE_PICKER_W: f32 = 190.;
/// Typing pause before a search starts.
pub(crate) const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);

/// `Label value ▾` trigger button styled like the mockup.
pub(crate) fn trigger(id: &'static str, label: &'static str, value: SharedString, cx: &App) -> Button {
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

/// A button of a segmented control (`Sessions | Usage`, the Usage ranges, the chart metric): the
/// picked one is white on the control's grey.
pub(crate) fn segment(id: impl Into<ElementId>, label: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h_full()
        .px(px(9.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .text_size(px(12.))
        .cursor_pointer()
        .when(on, |el| el.bg(style::white()).text_color(style::text()).font_weight(FontWeight::SEMIBOLD).border_1().border_color(style::control_border()))
        .when(!on, |el| el.text_color(style::text_secondary()).hover(|s| s.text_color(style::text())))
        .child(label.into())
}

/// The grey frame of a segmented control.
pub(crate) fn segmented() -> Div {
    h_flex().flex_none().h(px(CONTROL_H)).p(px(2.)).gap(px(1.)).rounded(px(6.)).bg(style::segment_bg())
}

/// A thin upright line between groups of controls.
fn separator() -> Div {
    div().flex_none().w(px(1.)).h(px(16.)).mx(px(2.)).bg(style::border())
}

/// A button in the toolbar with a word or an icon on it (`Custom…`, rescan): lit like the search
/// box's toggles while it is on.
fn tool_button(id: &'static str, on: bool, cx: &App) -> Button {
    let (bg, fg) = if on { (style::toggle_on_bg(), style::toggle_on_fg()) } else { (style::white(), style::text()) };
    let hover = if on { bg } else { style::control_hover() };
    Button::new(id)
        .custom(ButtonCustomVariant::new(cx).color(bg).foreground(fg).hover(hover).active(hover))
        .small()
        .h(px(CONTROL_H))
        .px(px(8.))
        .border_1()
        .border_color(if on { style::toggle_on_fg().opacity(0.35) } else { style::control_border() })
        .rounded(px(5.))
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

pub(crate) fn menu_item(
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
    pub(crate) fn current_filter(&self, cx: &App) -> Filter {
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

    pub fn set_range(&mut self, range: RangePreset, cx: &mut Context<Self>) {
        self.range = range;
        let mut filter = self.current_filter(cx);
        (filter.since_ms, filter.until_ms) = range.bounds_on(self.usage_day, self.custom_range);
        self.apply_filter(filter, cx);
        // the range is saved even when the filter came out the same (Custom over the same days)
        self.persist(cx);
    }

    /// A range picked in the date picker: both days included.
    pub fn set_custom_range(&mut self, start: NaiveDate, end: NaiveDate, cx: &mut Context<Self>) {
        self.custom_range = Some((start, end));
        self.set_range(RangePreset::Custom, cx);
    }

    pub fn toggle_kind(&mut self, kind: Kind, cx: &mut Context<Self>) {
        let mut filter = self.current_filter(cx);
        filter.kinds.toggle(kind);
        self.apply_filter(filter, cx);
    }

    /// Only sessions whose main model is `model` (a model id), or all of them. Kept without its
    /// date or `[1m]`: the session table and Usage share it, and Usage names models that way.
    pub fn set_model_filter(&mut self, model: Option<SharedString>, cx: &mut Context<Self>) {
        let mut filter = self.current_filter(cx);
        filter.model = model.map(|m| normalize(&m).to_string().into());
        self.apply_filter(filter, cx);
    }

    pub fn toggle_empty_sessions(&mut self, cx: &mut Context<Self>) {
        let mut filter = self.current_filter(cx);
        filter.show_empty = !filter.show_empty;
        self.apply_filter(filter, cx);
    }

    pub(crate) fn apply_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        // Project, Kind and Model are the Usage view's filters too
        let usage_changed = !crate::usage_view::same_filter(&crate::usage_view::usage_filter(&filter), &crate::usage_view::usage_filter(&self.current_filter(cx)));
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
            // the search looks into the filtered sessions only, so it has to look again (once the
            // table is back, when this came from the Usage view)
            if self.searching() {
                match self.view {
                    MainView::Sessions => self.run_search(cx),
                    MainView::Usage => self.search_went_stale(crate::StaleSearch::Rerun),
                }
            }
        }
        if usage_changed {
            self.refresh_usage(cx);
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
        self.search_refresh = None;
        self.query = self.search.read(cx).value().trim().to_string().into();
        self.matcher = self.search_opts.matcher(&self.query);
        // the conversation is marked when it is read: read it again only for another search
        // (this runs on filter changes and reloads too, with the same one)
        let pattern = self.matcher.as_ref().map(|m| m.pattern().to_string());
        if pattern != self.conversation_pattern && let Some(row) = self.selected.clone() {
            self.read_conversation(&row);
        }
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

    /// A reload changed the rows under the results on screen: search for the same again, without
    /// the user noticing. The table keeps the old results and the status bar their summary until
    /// the new ones are all in; they then replace them in one go, and the table is not scrolled
    /// (`on_refresh_events`). When the results on screen are not those of the box (it changed,
    /// or the first results have not come yet) this is a new search.
    pub(crate) fn refresh_search(&mut self, cx: &mut Context<Self>) {
        let typed = self.search.read(cx).value().trim().to_string();
        let shown = self.table.read(cx).delegate().search_active() && (!self.search_pending || self.search_refresh.is_some());
        if typed != self.query.as_ref() || !shown {
            self.run_search(cx);
            return;
        }
        let targets = self.table.read(cx).delegate().search_targets();
        self.search_seq = self.searcher.search(self.query.to_string(), self.search_opts.clone(), targets);
        self.search_pending = true;
        self.search_refresh = Some(Vec::new());
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
        if self.search_refresh.is_some() {
            self.on_refresh_events(current, cx);
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

    /// Events of a refresh (`refresh_search`): hold the hits until it ends, then swap them in.
    /// A session that is still found stays selected, and the table stays where it was scrolled.
    fn on_refresh_events(&mut self, events: Vec<SearchEvent>, cx: &mut Context<Self>) {
        let mut ended = false;
        for ev in events {
            match ev {
                SearchEvent::Hits { hits, .. } => self.search_refresh.get_or_insert_default().extend(hits),
                SearchEvent::Done { summary, .. } => {
                    println!("search {:?} ({:?}): {summary:?}", self.query, self.search_opts.mode);
                    self.search_summary = Some(summary);
                    self.search_error = None;
                    ended = true;
                }
                SearchEvent::Failed { message, .. } => {
                    eprintln!("search: {message}");
                    self.search_error = Some(message.into());
                    ended = true;
                }
            }
        }
        if !ended {
            return;
        }
        let hits = self.search_refresh.take().unwrap_or_default();
        self.search_pending = false;
        self.table.update(cx, |table, _| table.delegate_mut().replace_hits(hits));
        self.relocate_selection(false, cx);
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
                if preset == RangePreset::Custom {
                    menu = menu.separator();
                }
                let this = this.clone();
                menu = menu.item(menu_item(preset.menu_label().into(), "".into(), false, preset == current, move |cx| {
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
        let favorites_only = self.current_filter(cx).favorites_only;
        move |menu, _, _| {
            let mut menu = menu.min_w(px(160.));
            for kind in Kind::ALL {
                let this = this.clone();
                let count = counts[kind as usize].to_string();
                menu = menu.item(menu_item(kind.short().into(), count.into(), true, kinds.contains(kind), move |cx| {
                    this.update(cx, |app, cx| app.toggle_kind(kind, cx)).ok();
                }));
            }
            menu.separator().item(menu_item("★ Favorites only".into(), "".into(), false, favorites_only, {
                let this = this.clone();
                move |cx| { this.update(cx, |app, cx| app.toggle_favorites_only(cx)).ok(); }
            }))
        }
    }

    fn model_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let current: Option<SharedString> = self.current_filter(cx).model.map(|m| normalize(&m).to_string().into());
        let mut entries = self.models.clone();
        // a saved pick whose sessions are gone (or not loaded yet) stays listed, so it can be undone
        if let Some(model) = current.as_ref().filter(|m| !entries.iter().any(|e| e.model == **m)) {
            entries.push(ModelEntry { model: model.clone(), name: display_name(model).into(), count: 0 });
        }
        move |menu, _, _| {
            let all = this.clone();
            let mut menu = menu.min_w(px(180.)).max_h(px(420.)).scrollable(true).item(menu_item("All".into(), "".into(), false, current.is_none(), move |cx| {
                all.update(cx, |app, cx| app.set_model_filter(None, cx)).ok();
            }));
            if !entries.is_empty() {
                menu = menu.separator();
            }
            for entry in &entries {
                let (this, model) = (this.clone(), entry.model.clone());
                let checked = current.as_ref() == Some(&entry.model);
                menu = menu.item(menu_item(entry.name.clone(), entry.count.to_string().into(), true, checked, move |cx| {
                    this.update(cx, |app, cx| app.set_model_filter(Some(model.clone()), cx)).ok();
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

    /// `Sessions | Usage`, then the controls of the view on screen.
    pub fn render_toolbar(&self, cx: &mut Context<Self>) -> Div {
        let bar = h_flex()
            .w_full()
            .h(px(TOOLBAR_H))
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .bg(style::toolbar_bg())
            .border_b_1()
            .border_color(style::border_soft())
            .child(self.render_view_switch(cx))
            .child(separator());
        match self.view {
            MainView::Sessions => self.render_session_controls(bar, cx),
            MainView::Usage => self.render_usage_controls(bar, cx),
        }
    }

    fn render_view_switch(&self, cx: &mut Context<Self>) -> Div {
        let item = |id: &'static str, label: &'static str, view: MainView| {
            segment(id, label, self.view == view).on_click(cx.listener(move |this, _, window, cx| this.set_view(view, window, cx)))
        };
        segmented().child(item("view-sessions", "Sessions", MainView::Sessions)).child(item("view-usage", "Usage", MainView::Usage))
    }

    fn render_session_controls(&self, bar: Div, cx: &mut Context<Self>) -> Div {
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

        bar.child(trigger("project", "Project", project_label, cx).dropdown_menu(self.project_menu(cx)))
            .child(trigger("scope", "Scope", self.search_opts.scope.label().into(), cx).dropdown_menu(self.scope_menu(cx)))
            .child(trigger("range", "Updated", self.range.label().into(), cx).dropdown_menu(self.range_menu(cx)))
            .when(self.range == RangePreset::Custom, |el| {
                el.child(
                    div().w(px(DATE_PICKER_W)).child(
                        DatePicker::new(&self.date_picker).small().number_of_months(2).presets(date_presets(self.usage_day)),
                    ),
                )
            })
            .child(trigger("kind", "Kind", format!("{}{}", filter.kinds.label(), if filter.favorites_only { " · ★" } else { "" }).into(), cx).dropdown_menu(self.kind_menu(cx)))
            .child(trigger("model", "Model", filter.model.as_ref().map_or_else(|| "All".into(), |m| display_name(m).into()), cx).dropdown_menu(self.model_menu(cx)))
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

/// Shortcuts beside the calendar of the Custom range; `today` is a usage day (from 4:00).
fn date_presets(today: NaiveDate) -> Vec<DateRangePreset> {
    let month = today.with_day(1).unwrap_or(today);
    let last_month = month.checked_sub_months(Months::new(1)).unwrap_or(month);
    let back = |days: u64| today.checked_sub_days(Days::new(days)).unwrap_or(today);
    vec![
        DateRangePreset::range("This month", month, today),
        DateRangePreset::range("Last month", last_month, month.pred_opt().unwrap_or(month)),
        DateRangePreset::range("Last 7 days", back(6), today),
        DateRangePreset::range("Last 30 days", back(29), today),
    ]
}

/// The Usage toolbar's Model button, and its `as of` with the refresh button (tests).
pub const USAGE_MODEL_SELECTOR: &str = "usage-toolbar-model";
pub const USAGE_END_SELECTOR: &str = "usage-toolbar-end";

impl AppRoot {
    /// The Usage view's toolbar: ranges, `Custom…` (and its picker), Project / Kind / Model, and
    /// on the right when the data is from and a button that scans now. Too narrow a window for
    /// one row (1,140 px, 1,330 with the date picker) wraps it onto a second;
    /// `as of` and the refresh button stay right-aligned on whichever row they land.
    fn render_usage_controls(&self, bar: Div, cx: &mut Context<Self>) -> Div {
        let u = &self.usage_view;
        let filter = self.current_filter(cx);
        let mut ranges = segmented();
        for (ix, (range, label)) in RANGE_BUTTONS.iter().copied().enumerate() {
            ranges = ranges.child(segment(("usage-range", ix), label, u.range == range).on_click(cx.listener(move |this, _, _, cx| this.set_usage_range(range, cx))));
        }
        // a day clicked on the chart: its chip, ✕ goes back
        if let (UsageRange::Day, Some(day)) = (u.range, u.day) {
            ranges = ranges.child(
                segment("usage-day", format!("{} ✕", fmt_day_in(day, self.usage_day.year())), true)
                    .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Back to the range before (Esc)").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.leave_day(cx))),
            );
        }
        let custom = u.range == UsageRange::Custom;
        let project = match &filter.project {
            Some(key) => u.projects.iter().find(|p| p.key == key.as_ref()).map_or_else(|| key.clone(), |p| p.name.clone().into()),
            None => "All".into(),
        };
        let as_of: SharedString = match (self.scan_in_flight, self.scan_record.done_at) {
            (true, _) => "scanning…".into(),
            (false, Some(at)) => format!("as of {}", at.format("%H:%M")).into(),
            (false, None) => "".into(),
        };
        bar.h_auto()
            .min_h(px(TOOLBAR_H))
            .py(px(4.))
            .flex_wrap()
            .child(ranges)
            .child(
                tool_button("usage-custom", custom, cx)
                    .tooltip("Pick the first and last day")
                    .child(div().text_size(px(12.)).child("Custom…"))
                    .on_click(cx.listener(|this, _, _, cx| this.set_usage_range(UsageRange::Custom, cx))),
            )
            .when(custom, |el| {
                el.child(div().w(px(DATE_PICKER_W)).child(DatePicker::new(&u.date_picker).small().number_of_months(2).presets(date_presets(self.usage_day))))
            })
            .child(separator())
            .child(trigger("usage-project", "Project", project, cx).dropdown_menu(self.usage_project_menu(cx)))
            .child(trigger("usage-kind", "Kind", usage_kind_label(&filter).into(), cx).dropdown_menu(self.usage_kind_menu(cx)))
            .child(div().debug_selector(|| USAGE_MODEL_SELECTOR.into()).child(trigger("usage-model", "Model", filter.model.as_ref().map_or_else(|| "All".into(), |m| display_name(m).into()), cx).dropdown_menu(self.usage_model_menu(cx))))
            .child(
                h_flex()
                    .debug_selector(|| USAGE_END_SELECTOR.into())
                    .ml_auto()
                    .gap(px(6.))
                    .child(div().flex_none().text_size(px(11.)).text_color(style::muted()).child(as_of))
                    .child(
                        tool_button("usage-rescan", false, cx)
                            .w(px(26.))
                            .px(px(0.))
                            .tooltip("Scan for new calls now")
                            .child(Icon::new(IconName::RotateCw).size(px(13.)).text_color(style::label()))
                            .disabled(self.scan_in_flight)
                            .on_click(cx.listener(|this, _, _, cx| this.rescan(false, cx))),
                    ),
            )
    }

    /// Every project with calls in the ledger, including those whose transcripts are all gone;
    /// hidden ones too (Usage counts them).
    fn usage_project_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let current = self.current_filter(cx).project;
        let mut projects: Vec<(SharedString, SharedString, usize)> = self.usage_view.projects.iter().map(|p| (p.key.clone().into(), p.name.clone().into(), p.sessions)).collect();
        // a pick without calls in the ledger stays listed, so it can be undone
        if let Some(key) = current.as_ref().filter(|k| !projects.iter().any(|p| p.0 == **k)) {
            projects.insert(0, (key.clone(), key.clone(), 0));
        }
        let total: usize = projects.iter().map(|p| p.2).sum();
        move |menu, _, _| {
            let all = this.clone();
            let mut menu = menu.min_w(px(248.)).max_h(px(420.)).scrollable(true).item(menu_item("All projects".into(), total.to_string().into(), true, current.is_none(), move |cx| {
                all.update(cx, |app, cx| app.set_project(None, cx)).ok();
            }));
            if !projects.is_empty() {
                menu = menu.separator();
            }
            for (key, name, sessions) in &projects {
                let (this, pick) = (this.clone(), key.clone());
                menu = menu.item(menu_item(name.clone(), sessions.to_string().into(), true, current.as_ref() == Some(key), move |cx| {
                    let pick = pick.clone();
                    this.update(cx, |app, cx| app.set_project(Some(pick), cx)).ok();
                }));
            }
            menu
        }
    }

    /// CLI, SDK and BG with their sessions in the ledger; History means nothing here.
    fn usage_kind_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let kinds = self.current_filter(cx).kinds;
        let counts = self.usage_view.kind_counts;
        move |menu, _, _| {
            let mut menu = menu.min_w(px(160.));
            for (ix, kind) in [Kind::Cli, Kind::Sdk, Kind::Background].into_iter().enumerate() {
                let this = this.clone();
                menu = menu.item(menu_item(kind.short().into(), counts[ix].to_string().into(), true, kinds.contains(kind), move |cx| {
                    this.update(cx, |app, cx| app.toggle_kind(kind, cx)).ok();
                }));
            }
            menu
        }
    }

    /// Every model in the ledger and its calls: Usage filters each call by its own model.
    fn usage_model_menu(&self, cx: &Context<Self>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let this = cx.entity().downgrade();
        let current = self.current_filter(cx).model;
        let picked = current.as_ref().map(|m| normalize(m).to_string());
        let mut entries: Vec<(String, u64)> = self.usage_view.models.clone();
        if let Some(model) = picked.as_ref().filter(|m| !entries.iter().any(|e| e.0 == **m)) {
            entries.push((model.clone(), 0));
        }
        move |menu, _, _| {
            let all = this.clone();
            let mut menu = menu.min_w(px(200.)).max_h(px(420.)).scrollable(true).item(menu_item("All".into(), "".into(), false, current.is_none(), move |cx| {
                all.update(cx, |app, cx| app.set_model_filter(None, cx)).ok();
            }));
            if !entries.is_empty() {
                menu = menu.separator();
            }
            for (model, calls) in &entries {
                let (this, pick) = (this.clone(), SharedString::from(model.clone()));
                let hint = format!("{} calls", crate::resident::thousands(*calls));
                menu = menu.item(menu_item(display_name(model).into(), hint.into(), true, picked.as_ref() == Some(model), move |cx| {
                    this.update(cx, |app, cx| app.set_model_filter(Some(pick.clone()), cx)).ok();
                }));
            }
            menu
        }
    }
}

/// The Kind button's value in the Usage view: CLI, SDK, BG only.
fn usage_kind_label(filter: &Filter) -> String {
    let on: Vec<&str> = [Kind::Cli, Kind::Sdk, Kind::Background].into_iter().filter(|k| filter.kinds.contains(*k)).map(|k| k.short()).collect();
    match on.len() {
        0 => "None".into(),
        3 => "All".into(),
        _ => on.join(", "),
    }
}
