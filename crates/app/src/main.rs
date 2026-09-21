//! Agent Session Manager — macOS desktop app listing Claude Code sessions.
//!
//! Layout follows the Claude Design mockup: toolbar (dropdown filters + search), session table,
//! detail panel with Metadata / Conversation tabs, status bar. Sessions come from `crates/core`
//! through `store` (index load, then a streaming incremental scan). Set `ASM_SPIKE_AUTOPLAY=1`
//! to scroll / select / filter / sort automatically after launch (used by `make screenshot`),
//! `ASM_SEARCH_AUTOPLAY=1` to run a scripted search session (`make screenshot-search`);
//! `ASM_CLAUDE_DIR`, `ASM_DB` and `ASM_STATE` point the app at other data.

mod detail;
mod model;
mod search;
mod settings;
mod store;
mod style;
mod table;
mod toolbar;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_session_core::claude::paths::{ClaudeDir, default_db_path};
use agent_session_core::format::human_size;
use agent_session_core::index::ScanReport;
use agent_session_core::search::Matcher;

use gpui_kit::component::input::{Escape, InputEvent, InputState};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::table::{ColumnSort, DataTable, TableEvent, TableState};
use gpui_kit::component::{ResizableState, Root, Sizable, h_resizable, resizable_panel, v_flex};
use gpui_kit::*;

use crate::detail::{DETAIL_W, DetailState, DetailTab};
use crate::model::{DisplayMode, Filter, Kind, ProjectEntry, RangePreset, SessionRow, kind_counts, project_entries, split_hidden};
use crate::search::{SearchOptions, SearchSummary, Searcher};
use crate::settings::{SortDir, ViewState, default_state_path};
use crate::store::{Store, StoreEvent, StoreRequest};
use crate::table::{PREVIEW_MIN_W, ROW_H, SessionTableDelegate};

gpui_kit::actions!(
    app,
    [
        Quit,
        ToggleDetail,
        FocusSearch,
        ToggleRegex,
        ToggleCase,
        NextHit,
        PrevHit,
        Preview1,
        Preview2,
        Preview3,
        Preview4,
        Rescan,
        RebuildIndex,
        ToggleEmptySessions,
        HideSelectedProject
    ]
);

const WINDOW_W: f32 = 1440.;
const WINDOW_H: f32 = 900.;
/// Space the table needs besides its columns: trailing empty column + vertical scrollbar.
const TABLE_CHROME_W: f32 = 16.;

/// What the index is doing, shown in the middle of the status bar.
#[derive(Clone, PartialEq)]
enum IndexStatus {
    Loading,
    Scanning { done: usize, total: usize },
    Idle(SharedString),
    Failed(SharedString),
}

pub struct AppRoot {
    table: Entity<TableState<SessionTableDelegate>>,
    search: Entity<InputState>,
    resizable: Entity<ResizableState>,
    store: Store,
    projects: Vec<ProjectEntry>,
    /// Hidden projects, listed apart in the Project dropdown so they can be shown again.
    hidden_projects: Vec<ProjectEntry>,
    /// Sessions per kind (`Kind as usize`), shown in the Kind dropdown.
    kind_counts: [usize; Kind::ALL.len()],
    searcher: Searcher,
    search_opts: SearchOptions,
    /// The query being searched for (trimmed); empty when no search is on.
    query: SharedString,
    /// Marks the hits of `query` in the detail panel.
    matcher: Option<Matcher>,
    /// Events of any other search are stale.
    search_seq: u64,
    /// The table still shows what came before `search_seq`; its first event swaps the results.
    search_pending: bool,
    search_summary: Option<SearchSummary>,
    search_error: Option<SharedString>,
    range: RangePreset,
    selected: Option<Arc<SessionRow>>,
    /// Selection to restore once its row shows up (saved state, or a row filtered away and back).
    wanted_id: Option<SharedString>,
    detail: DetailState,
    detail_visible: bool,
    status: SharedString,
    index_status: IndexStatus,
    db_info: SharedString,
    launched: Instant,
    first_rows_ms: Option<u128>,
    state_path: PathBuf,
    saved: ViewState,
    _subscriptions: Vec<Subscription>,
    _events: Task<()>,
    _search_events: Task<()>,
    _debounce: Option<Task<()>>,
    _autoplay: Option<Task<()>>,
}

fn scan_summary(report: &ScanReport) -> SharedString {
    let changed = report.files_parsed + report.files_appended + report.files_rebuilt;
    let secs = report.elapsed.as_secs_f64();
    if changed == 0 && report.files_missing == 0 && report.history_sessions == 0 {
        format!("index up to date · {} files checked in {secs:.2}s", report.files_seen).into()
    } else {
        format!("indexed {changed} of {} files in {secs:.2}s", report.files_seen).into()
    }
}

impl AppRoot {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state_path = default_state_path();
        let saved = ViewState::load(&state_path);
        let mut delegate = SessionTableDelegate::new();
        delegate.mode = saved.mode;
        delegate.set_filter(Filter {
            project: saved.project_key.clone().map(Into::into),
            since_ms: saved.range.since_ms(),
            show_empty: saved.show_empty,
            kinds: saved.kinds,
            hidden_projects: saved.hidden_projects.iter().cloned().map(Into::into).collect(),
        });
        if let Some((key, dir)) = &saved.sort {
            delegate.set_sort(key, dir.to_column());
        }
        // a selected column only hides the selected row; header clicks are for sorting
        let table = cx.new(|cx| TableState::new(delegate, window, cx).col_selectable(false));

        // escape is handled by `clear_search` (the box propagates it), which also hands the focus back
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions"));
        let resizable = cx.new(|_| ResizableState::default());

        let subscriptions = vec![
            cx.subscribe_in(&table, window, Self::on_table_event),
            cx.subscribe_in(&search, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => this.schedule_search(cx),
                InputEvent::PressEnter { .. } => {
                    this.run_search(cx);
                    this.focus_table(window, cx);
                }
                _ => {}
            }),
            // header clicks sort inside the table; persist whatever changed once it settles
            cx.observe(&table, |this, _, cx| this.persist(cx)),
        ];

        let (store, events) = Store::spawn(ClaudeDir::from_env(), default_db_path());
        let pump = cx.spawn(async move |this, cx| {
            while let Ok(first) = events.recv().await {
                let mut batch = vec![first];
                while let Ok(ev) = events.try_recv() {
                    batch.push(ev);
                }
                if this.update(cx, |this, cx| this.on_store_events(batch, cx)).is_err() {
                    break;
                }
            }
        });

        let (searcher, search_events) = Searcher::spawn(default_db_path());
        let search_pump = cx.spawn(async move |this, cx| {
            while let Ok(first) = search_events.recv().await {
                let mut batch = vec![first];
                while let Ok(ev) = search_events.try_recv() {
                    batch.push(ev);
                }
                if this.update(cx, |this, cx| this.on_search_events(batch, cx)).is_err() {
                    break;
                }
            }
        });

        let mut this = Self {
            table,
            search,
            resizable,
            store,
            projects: Vec::new(),
            hidden_projects: Vec::new(),
            kind_counts: [0; Kind::ALL.len()],
            searcher,
            search_opts: saved.search.clone(),
            query: "".into(),
            matcher: None,
            search_seq: 0,
            search_pending: false,
            search_summary: None,
            search_error: None,
            range: saved.range,
            selected: None,
            wanted_id: saved.selected_id.clone().map(Into::into),
            detail: DetailState::default(),
            detail_visible: saved.detail_visible,
            status: "".into(),
            index_status: IndexStatus::Loading,
            db_info: "".into(),
            launched: Instant::now(),
            first_rows_ms: None,
            state_path,
            saved,
            _subscriptions: subscriptions,
            _events: pump,
            _search_events: search_pump,
            _debounce: None,
            _autoplay: None,
        };
        if std::env::var_os("ASM_SPIKE_AUTOPLAY").is_some() {
            this._autoplay = Some(this.autoplay(window, cx));
        } else if std::env::var_os("ASM_SEARCH_AUTOPLAY").is_some() {
            this._autoplay = Some(this.search_autoplay(window, cx));
        }
        this
    }

    /// Apply a batch of store events with a single table recompute.
    fn on_store_events(&mut self, batch: Vec<StoreEvent>, cx: &mut Context<Self>) {
        let mut streamed = Vec::new();
        let mut data_changed = false;
        let mut reloaded = false;
        for ev in batch {
            match ev {
                StoreEvent::Loaded { sessions, elapsed } => {
                    let rows: Vec<Arc<SessionRow>> = sessions.iter().map(|s| Arc::new(SessionRow::from_summary(s))).collect();
                    streamed.clear();
                    if !rows.is_empty() && self.first_rows_ms.is_none() {
                        self.first_rows_ms = Some(self.launched.elapsed().as_millis());
                        println!("{} sessions on screen {}ms after launch (index read {elapsed:?})", rows.len(), self.launched.elapsed().as_millis());
                    }
                    self.table.update(cx, |table, _| table.delegate_mut().set_rows(rows));
                    data_changed = true;
                    reloaded = true;
                }
                StoreEvent::Session(s) => streamed.push(Arc::new(SessionRow::from_summary(&s))),
                StoreEvent::Progress(p) if p.done < p.total => self.index_status = IndexStatus::Scanning { done: p.done, total: p.total },
                StoreEvent::Progress(_) => {}
                StoreEvent::ScanDone { report, stats } => {
                    println!("scan: {report:?}");
                    self.index_status = IndexStatus::Idle(scan_summary(&report));
                    self.db_info = format!("{} messages · DB {}", stats.messages, human_size(stats.db_bytes)).into();
                    data_changed = true;
                }
                StoreEvent::Messages { session_id, messages } => {
                    if self.selected.as_ref().is_some_and(|row| row.id == session_id) {
                        self.detail.set_messages(messages, self.matcher.as_ref());
                    }
                }
                StoreEvent::Failed(message) => {
                    eprintln!("store: {message}");
                    self.index_status = IndexStatus::Failed(message.into());
                }
            }
        }
        if !streamed.is_empty() {
            self.first_rows_ms.get_or_insert(self.launched.elapsed().as_millis());
            self.table.update(cx, |table, _| table.delegate_mut().upsert(streamed));
            data_changed = true;
        }
        if data_changed {
            self.after_rows_changed(cx);
        }
        // a reload follows a scan that changed the index: the results on screen may be out of date
        if reloaded && self.searching() {
            self.run_search(cx);
        }
        cx.notify();
    }

    /// Rows or the filter changed: refresh the dropdown counts and keep the selection on its session.
    pub(crate) fn after_rows_changed(&mut self, cx: &mut Context<Self>) {
        let delegate = self.table.read(cx).delegate();
        let (show_empty, hidden) = (delegate.filter().show_empty, &delegate.filter().hidden_projects);
        let listed = || delegate.rows.iter().filter(|r| show_empty || r.prompts > 0);
        (self.projects, self.hidden_projects) = split_hidden(project_entries(listed()), hidden);
        self.kind_counts = kind_counts(listed().filter(|r| !hidden.contains(&r.project_key)));
        let wanted = self.wanted_id.clone().or_else(|| self.selected.as_ref().map(|r| r.id.clone()));
        let position = wanted.as_ref().and_then(|id| delegate.position_of(id));
        let has_rows = !delegate.ix.is_empty();
        // While a scan streams rows in, the top row keeps changing (and a selection restored from
        // the saved state may still be on its way): pick a default only once things settle.
        let scanning = matches!(self.index_status, IndexStatus::Scanning { .. });
        let waiting = self.selected.is_none() && (scanning || (self.wanted_id.is_some() && self.index_status == IndexStatus::Loading));
        match position {
            Some(row_ix) => self.select_row(row_ix, cx),
            None if waiting => {}
            None if has_rows => self.select_row(0, cx),
            None => self.clear_selection(cx),
        }
        self.table.update(cx, |_, cx| cx.notify());
    }

    fn select_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        let row = self.table.update(cx, |table, cx| {
            let row = table.delegate().ix.get(row_ix).map(|_| table.delegate().row(row_ix).clone());
            if row.is_some() && table.selected_row() != Some(row_ix) {
                table.set_selected_row(row_ix, cx);
            }
            row
        });
        if let Some(row) = row {
            let same_session = self.selected.as_ref().is_some_and(|old| old.id == row.id);
            if !same_session {
                self.detail.clear();
                self.store.request(StoreRequest::Messages(row.id.to_string()));
            }
            self.wanted_id = None;
            self.selected = Some(row);
            self.persist(cx);
        }
        cx.notify();
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| table.clear_selection(cx));
        self.selected = None;
        self.wanted_id = None;
        self.detail.clear();
    }

    /// Snapshot of everything worth restoring on the next launch.
    fn view_state(&self, cx: &App) -> ViewState {
        let delegate = self.table.read(cx).delegate();
        ViewState {
            mode: delegate.mode,
            // the Matches column is gone after a restart, and so is a sort by it
            sort: delegate.sort_state().filter(|(key, _)| key != "matches").and_then(|(key, sort)| Some((key.to_string(), SortDir::from_column(sort)?))),
            project_key: delegate.filter().project.as_ref().map(|p| p.to_string()),
            range: self.range,
            show_empty: delegate.filter().show_empty,
            kinds: delegate.filter().kinds,
            hidden_projects: delegate.filter().hidden_projects.iter().map(|p| p.to_string()).collect(),
            detail_visible: self.detail_visible,
            selected_id: self.selected.as_ref().map(|r| r.id.to_string()).or_else(|| self.wanted_id.as_ref().map(|id| id.to_string())),
            search: self.search_opts.clone(),
        }
    }

    pub(crate) fn persist(&mut self, cx: &App) {
        let state = self.view_state(cx);
        if state == self.saved {
            return;
        }
        if let Err(e) = state.save(&self.state_path) {
            eprintln!("cannot save {}: {e:#}", self.state_path.display());
        }
        self.saved = state;
    }

    fn rescan(&mut self, rebuild: bool, cx: &mut Context<Self>) {
        self.index_status = IndexStatus::Loading;
        self.store.request(if rebuild { StoreRequest::Rebuild } else { StoreRequest::Scan });
        cx.notify();
    }

    fn on_table_event(&mut self, _: &Entity<TableState<SessionTableDelegate>>, ev: &TableEvent, _: &mut Window, cx: &mut Context<Self>) {
        match ev {
            TableEvent::SelectRow(ix) => self.select_row(*ix, cx),
            TableEvent::DoubleClickedRow(ix) => {
                self.detail_visible = true;
                self.select_row(*ix, cx);
            }
            _ => {}
        }
    }

    /// Scripted interaction for screenshots: scroll deep + select, then filter / sort / switch tab.
    fn autoplay(&self, window: &Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            this.update_in(cx, |this, _, cx| {
                let target = this.table.read(cx).delegate().ix.len() / 2;
                this.table.update(cx, |table, cx| table.scroll_to_row(target, cx));
                this.select_row(target, cx);
                this.status = format!("autoplay: scrolled to row {target}").into();
                cx.notify();
            })
            .ok();
            cx.background_executor().timer(Duration::from_secs(3)).await;
            this.update_in(cx, |this, _, cx| {
                this.set_project(None, cx);
                this.set_range(RangePreset::Days30, cx);
                let delegate = this.table.read(cx).delegate();
                let visible = (0..delegate.ix.len()).map(|i| delegate.row(i));
                let busiest = project_entries(visible).into_iter().max_by_key(|p| p.count);
                let name = busiest.as_ref().map(|p| p.name.clone()).unwrap_or_default();
                this.set_project(busiest.map(|p| p.key), cx);
                this.table.update(cx, |table, cx| {
                    table.delegate_mut().set_sort("prompts", ColumnSort::Descending);
                    table.refresh(cx);
                    table.scroll_to_row(0, cx);
                });
                this.select_row(0, cx);
                this.set_detail_tab(DetailTab::Metadata, cx);
                this.status = format!("autoplay: {name} · 30 days · sorted by Prompts desc").into();
                cx.notify();
            })
            .ok();
        })
    }

    /// Scripted search session for screenshots. After every step the search is given time to
    /// settle, then `autoplay-search: <step>` is printed and the view held for a capture.
    /// `ASM_AUTOPLAY_QUERY` / `_CJK` / `_REGEX` replace the default queries.
    fn search_autoplay(&self, window: &Window, cx: &mut Context<Self>) -> Task<()> {
        let query = |name: &str, default: &str| std::env::var(name).ok().filter(|q| !q.is_empty()).unwrap_or_else(|| default.to_string());
        let text = query("ASM_AUTOPLAY_QUERY", "session table");
        let cjk = query("ASM_AUTOPLAY_CJK", "搜索");
        let regex = query("ASM_AUTOPLAY_REGEX", "cargo (test|build) -p");
        type Step = Box<dyn FnOnce(&mut AppRoot, &mut Window, &mut Context<AppRoot>)>;
        let steps: Vec<(&str, Step)> = vec![
            ("text-search", Box::new(move |this, window, cx| this.set_query(&text, window, cx))),
            ("next-hit", Box::new(|this, _, cx| this.step_hit(true, cx))),
            ("cjk-two-chars", Box::new(move |this, window, cx| this.set_query(&cjk, window, cx))),
            (
                "regex",
                Box::new(move |this, window, cx| {
                    this.toggle_regex(cx);
                    this.set_query(&regex, window, cx);
                }),
            ),
            (
                "regex-tools",
                Box::new(|this, _, cx| {
                    this.search_opts.scope.tools = true;
                    this.search_options_changed(cx);
                }),
            ),
            (
                "sorted-by-matches",
                Box::new(|this, _, cx| {
                    this.table.update(cx, |table, cx| {
                        table.delegate_mut().set_sort("matches", ColumnSort::Descending);
                        table.refresh(cx);
                        table.scroll_to_row(0, cx);
                    });
                    this.after_rows_changed(cx);
                }),
            ),
            ("cleared", Box::new(|this, window, cx| this.clear_search(window, cx))),
        ];
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            for (name, step) in steps {
                if this.update_in(cx, |this, window, cx| step(this, window, cx)).is_err() {
                    return;
                }
                // past the debounce, then until the running search has reported
                cx.background_executor().timer(Duration::from_millis(400)).await;
                for _ in 0..80 {
                    let busy = this.update(cx, |this, _| this.searching() && this.search_summary.is_none() && this.search_error.is_none());
                    if !busy.unwrap_or(false) {
                        break;
                    }
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                }
                cx.background_executor().timer(Duration::from_millis(400)).await;
                println!("autoplay-search: {name}");
                cx.background_executor().timer(Duration::from_millis(1500)).await;
            }
        })
    }

    /// Put `query` into the search box and search for it (a programmatic `set_value` emits no
    /// change event, so the debounce a typed query goes through is skipped).
    fn set_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |input, cx| input.set_value(query.to_string(), window, cx));
        self.run_search(cx);
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.search.read(cx).focus_handle(cx).clone();
        handle.focus(window, cx);
    }

    /// Keyboard focus to the table: arrows move the selection, `n` / `shift-n` step through hits.
    fn focus_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.table.read(cx).focus_handle(cx).clone();
        handle.focus(window, cx);
    }

    /// Escape in the search box: drop the search and go back to the table.
    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |input, cx| input.set_value("", window, cx));
        self.run_search(cx);
        self.focus_table(window, cx);
    }

    /// Keep the Preview column filling whatever width the table panel has.
    fn sync_preview_width(&mut self, window: &Window, cx: &mut Context<Self>) {
        let viewport_w = window.viewport_size().width;
        let panel_w = self.resizable.read(cx).sizes().first().copied().filter(|w| *w > px(0.));
        let table_w = match panel_w {
            Some(w) => w,
            None if self.detail_visible => viewport_w - px(DETAIL_W),
            None => viewport_w,
        };
        let fixed_w = self.table.read(cx).delegate().fixed_cols_width();
        let preview_w = (table_w - px(fixed_w) - px(TABLE_CHROME_W)).max(px(PREVIEW_MIN_W));
        let current = self.table.read(cx).delegate().preview_width();
        if (current - preview_w).abs() > px(0.5) {
            self.table.update(cx, |table, cx| {
                table.delegate_mut().set_preview_width(preview_w);
                table.refresh(cx);
            });
        }
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> StatusBar {
        let delegate = self.table.read(cx).delegate();
        let (shown, total) = (delegate.ix.len(), delegate.rows.len());
        let index = match &self.index_status {
            IndexStatus::Loading => "loading index…".into(),
            IndexStatus::Scanning { done, total } => format!("indexing {done} / {total} files").into(),
            IndexStatus::Idle(summary) => summary.clone(),
            IndexStatus::Failed(message) => format!("index error: {message}").into(),
        };
        let (found, took) = self.search_status().unwrap_or_default();
        let status = if found.is_empty() { self.status.clone() } else { found };
        let center: SharedString = if status.is_empty() { index } else { format!("{status} · {index}").into() };
        let took = if took.is_empty() { String::new() } else { format!("{took} · ") };
        let first_rows = self.first_rows_ms.map(|ms| format!(" · first rows in {ms}ms")).unwrap_or_default();
        StatusBar::new()
            .h(px(22.))
            .py_0()
            .px(px(10.))
            .font_family(style::mono(cx))
            .text_size(px(11.))
            .text_color(style::head_fg())
            .left(format!("{shown} / {total} sessions"))
            .child(center)
            .right(format!("{took}{}{first_rows}", self.db_info))
    }
}

impl Render for AppRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_preview_width(window, cx);
        v_flex()
            .size_full()
            .bg(style::white())
            .text_color(style::text())
            .text_size(px(12.))
            .line_height(px(16.))
            .on_action(cx.listener(|this, _: &ToggleDetail, _, cx| {
                this.detail_visible = !this.detail_visible;
                this.persist(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Rescan, _, cx| this.rescan(false, cx)))
            .on_action(cx.listener(|this, _: &RebuildIndex, _, cx| this.rescan(true, cx)))
            .on_action(cx.listener(|this, _: &ToggleEmptySessions, _, cx| this.toggle_empty_sessions(cx)))
            .on_action(cx.listener(|this, _: &HideSelectedProject, _, cx| this.hide_selected_project(cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_action(cx.listener(|this, _: &Escape, window, cx| this.clear_search(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleRegex, _, cx| this.toggle_regex(cx)))
            .on_action(cx.listener(|this, _: &ToggleCase, _, cx| this.toggle_case(cx)))
            .on_action(cx.listener(|this, _: &NextHit, _, cx| this.step_hit(true, cx)))
            .on_action(cx.listener(|this, _: &PrevHit, _, cx| this.step_hit(false, cx)))
            .on_action(cx.listener(|this, _: &Preview1, _, cx| this.set_mode(DisplayMode::Title, cx)))
            .on_action(cx.listener(|this, _: &Preview2, _, cx| this.set_mode(DisplayMode::FirstPrompt, cx)))
            .on_action(cx.listener(|this, _: &Preview3, _, cx| this.set_mode(DisplayMode::LastPrompt, cx)))
            .on_action(cx.listener(|this, _: &Preview4, _, cx| this.set_mode(DisplayMode::LastMessage, cx)))
            .child(self.render_toolbar(cx))
            .child(
                div().flex_1().min_h_0().w_full().child(
                    h_resizable("main")
                        .with_state(&self.resizable)
                        .child(
                            resizable_panel()
                                .size(px(WINDOW_W - DETAIL_W))
                                .size_range(px(480.)..px(2400.))
                                .child(
                                    div()
                                        .size_full()
                                        .border_r_1()
                                        .border_color(style::border())
                                        .child(DataTable::new(&self.table).stripe(false).bordered(false).with_size(px(ROW_H))),
                                ),
                        )
                        .child(
                            resizable_panel()
                                .size(px(DETAIL_W))
                                .size_range(px(360.)..px(1200.))
                                .visible(self.detail_visible)
                                .child(self.render_detail(cx)),
                        ),
                ),
            )
            .child(self.render_status_bar(cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    app.run(move |cx| {
        gpui_kit::init(cx);
        style::install_theme(cx);
        cx.activate(true);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-\\", ToggleDetail, None),
            KeyBinding::new("cmd-f", FocusSearch, None),
            KeyBinding::new("alt-cmd-r", ToggleRegex, None),
            KeyBinding::new("alt-cmd-c", ToggleCase, None),
            KeyBinding::new("cmd-g", NextHit, None),
            KeyBinding::new("cmd-shift-g", PrevHit, None),
            // plain keys only while the table has the focus, never while typing in the search box
            KeyBinding::new("n", NextHit, Some("DataTable")),
            KeyBinding::new("shift-n", PrevHit, Some("DataTable")),
            KeyBinding::new("cmd-1", Preview1, None),
            KeyBinding::new("cmd-2", Preview2, None),
            KeyBinding::new("cmd-3", Preview3, None),
            KeyBinding::new("cmd-4", Preview4, None),
            KeyBinding::new("cmd-shift-r", RebuildIndex, None),
        ]);
        cx.set_menus([
            Menu::new("Agent Session Manager").items(vec![MenuItem::action("Quit", Quit)]),
            Menu::new("View").items(vec![
                MenuItem::action("Find", FocusSearch),
                MenuItem::action("Find Next Hit", NextHit),
                MenuItem::action("Find Previous Hit", PrevHit),
                MenuItem::action("Regex Search", ToggleRegex),
                MenuItem::action("Match Case (Regex)", ToggleCase),
                MenuItem::action("Toggle Detail Panel", ToggleDetail),
                MenuItem::separator(),
                MenuItem::action("Preview: Title", Preview1),
                MenuItem::action("Preview: First Prompt", Preview2),
                MenuItem::action("Preview: Last Prompt", Preview3),
                MenuItem::action("Preview: Last Message", Preview4),
                MenuItem::separator(),
                MenuItem::action("Show / Hide Sessions Without Prompts", ToggleEmptySessions),
                MenuItem::action("Hide Project of Selected Session", HideSelectedProject),
            ]),
            Menu::new("Index").items(vec![
                MenuItem::action("Scan for Changes", Rescan),
                MenuItem::action("Rebuild Index", RebuildIndex),
            ]),
        ]);

        let bounds = Bounds::centered(None, size(px(WINDOW_W), px(WINDOW_H)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Agent Session Manager".into()), ..Default::default() }),
            ..Default::default()
        };
        cx.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| AppRoot::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
