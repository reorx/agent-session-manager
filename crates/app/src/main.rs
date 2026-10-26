//! Agent Session Manager — macOS desktop app listing Claude Code sessions.
//!
//! Layout follows the Claude Design mockup: toolbar (dropdown filters + search), session table,
//! detail panel with Metadata / Conversation tabs, status bar. Sessions come from `crates/core`
//! through `store` (index load, then a streaming incremental scan). Set `ASM_SPIKE_AUTOPLAY=1`
//! to scroll / select / filter / sort automatically after launch (used by `make screenshot`),
//! `ASM_SEARCH_AUTOPLAY=1` to run a scripted search session (`make screenshot-search`),
//! `ASM_DETAIL_AUTOPLAY=1` to tour the Conversation tab (`make screenshot-detail`);
//! `ASM_CLAUDE_DIR`, `ASM_DB` and `ASM_STATE` point the app at other data.

mod actions;
mod conversation;
mod detail;
mod favorites;
#[cfg(test)]
mod keyboard_tests;
mod model;
mod search;
mod settings;
mod store;
mod style;
mod table;
mod toolbar;
mod turn_cards;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_session_core::claude::paths::{ClaudeDir, default_db_path};
use agent_session_core::format::{fmt_usd, human_size};
use agent_session_core::index::ScanReport;
use agent_session_core::search::Matcher;

use chrono::NaiveDate;
use gpui_kit::component::date_picker::{DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{Escape, InputEvent, InputState};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::table::{ColumnSort, DataTable, TableEvent, TableState};
use gpui_kit::component::{ResizableState, Root, Sizable, h_resizable, resizable_panel, v_flex};
use gpui_kit::*;

use crate::actions::{CopyResume, CopySessionId, ExportTranscript, HideProject, OpenProjectFolder, RevealTranscript};
use crate::conversation::{Card, ConversationView};
use crate::detail::{DETAIL_W, DetailState, DetailTab, Load};
use crate::model::{DisplayMode, Filter, Kind, ProjectEntry, RangePreset, SessionRow, kind_counts, project_entries, split_hidden};
use crate::search::{SearchMode, SearchOptions, SearchSummary, Searcher};
use crate::settings::{SortDir, ViewState, default_state_path};
use crate::store::{Store, StoreEvent, StoreRequest};
use crate::table::{PREVIEW_MIN_W, ROW_H, SessionTableDelegate};

gpui_kit::actions!(
    app,
    [
        ToggleFavorite,
        RevealFavoritesFolder,
        ChangeFavoritesFolder,
        DefaultFavoritesFolder,
        Quit,
        ToggleDetail,
        FocusSearch,
        ToggleRegex,
        ToggleCase,
        NextHit,
        PrevHit,
        NextTurn,
        PrevTurn,
        Preview1,
        Preview2,
        Preview3,
        Preview4,
        Rescan,
        RebuildIndex,
        ToggleEmptySessions
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

/// A session being opened in the detail panel, for the `detail:` timing line: from
/// `select_row` to the first frame that shows its conversation.
struct DetailTiming {
    short_id: SharedString,
    started: Instant,
    /// Transcript size, read and build time, once the conversation arrived.
    loaded: Option<(u64, Duration, Duration)>,
}

pub struct AppRoot {
    claude: ClaudeDir,
    favorites: agent_session_core::favorites::FavoritesDir,
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
    /// The Custom range once one was picked; until then Custom means this month so far.
    custom_range: Option<(NaiveDate, NaiveDate)>,
    /// Picks the Custom range; shown next to the Updated button while Custom is on.
    date_picker: Entity<DatePickerState>,
    selected: Option<Arc<SessionRow>>,
    /// Selection to restore once its row shows up (saved state, or a row filtered away and back).
    wanted_id: Option<SharedString>,
    detail: DetailState,
    /// `Conversation` events of any other read are stale.
    conversation_seq: u64,
    /// `Matcher::pattern()` the last read of the selected session was asked to mark; the same
    /// search run again (a filter change, a reload) does not read the transcript again.
    conversation_pattern: Option<String>,
    detail_timing: Option<DetailTiming>,
    detail_visible: bool,
    /// Where the last TXT export of this run went: the save dialog starts there next time.
    last_export_dir: Option<PathBuf>,
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
    #[cfg(test)]
    selection_event_count: Option<usize>,
}

/// Where the app finds its data and keeps its own.
pub struct AppPaths {
    pub claude: ClaudeDir,
    pub db: PathBuf,
    pub state: PathBuf,
    pub favorites: agent_session_core::favorites::FavoritesDir,
}

impl AppPaths {
    /// `ASM_CLAUDE_DIR`, `ASM_DB` and `ASM_STATE`, or their defaults.
    fn from_env() -> Self {
        AppPaths { claude: ClaudeDir::from_env(), db: default_db_path(), state: default_state_path(),
            favorites: agent_session_core::favorites::FavoritesDir {
                root: agent_session_core::config::favorites_dir(&default_db_path()).expect("read favorites config"),
                trash: agent_session_core::favorites::Trash::from_env(),
            }
        }
    }
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
    fn new(paths: AppPaths, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state_path = paths.state;
        let saved = ViewState::load(&state_path);
        let mut delegate = SessionTableDelegate::new(paths.claude.clone());
        delegate.mode = saved.mode;
        let (since_ms, until_ms) = saved.range.bounds(saved.custom_range);
        delegate.set_filter(Filter {
            project: saved.project_key.clone().map(Into::into),
            since_ms,
            until_ms,
            show_empty: saved.show_empty,
            kinds: saved.kinds,
            favorites_only: saved.favorites_only,
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
        let shown_range = saved.custom_range.unwrap_or_else(|| RangePreset::default_custom(chrono::Local::now().date_naive()));
        let date_picker = cx.new(|cx| {
            let mut picker = DatePickerState::range(window, cx).date_format("%Y-%m-%d");
            picker.set_date(shown_range, window, cx);
            picker
        });

        let mut observed_sort = table.read(cx).delegate().sort_state();
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
            // Header sorting changes row positions without changing TableState's selected
            // index. Keep the session selected, and record the sort BEFORE notifying again
            // through after_rows_changed so that this observer cannot feed itself forever.
            cx.observe(&table, move |this, table, cx| {
                let sort = table.read(cx).delegate().sort_state();
                if sort != observed_sort {
                    observed_sort = sort;
                    this.after_rows_changed(cx);
                }
                this.persist(cx);
            }),
            // a range is complete once both ends are picked (presets pick both at once)
            cx.subscribe_in(&date_picker, window, |this, _, ev: &DatePickerEvent, window, cx| {
                let DatePickerEvent::Change(date) = ev;
                if let (Some(start), Some(end)) = (date.start(), date.end()) {
                    this.set_custom_range(start, end, cx);
                }
                this.focus_table(window, cx);
            }),
        ];

        let (store, events) = Store::spawn(paths.claude.clone(), paths.db.clone(), paths.favorites.clone());
        let pump = cx.spawn_in(window, async move |this, cx| {
            while let Ok(first) = events.recv().await {
                let mut batch = vec![first];
                while let Ok(ev) = events.try_recv() {
                    batch.push(ev);
                }
                if this.update_in(cx, |this, window, cx| this.on_store_events(batch, window, cx)).is_err() {
                    break;
                }
            }
        });

        let (searcher, search_events) = Searcher::spawn(paths.db);
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
            claude: paths.claude,
            favorites: paths.favorites,
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
            custom_range: saved.custom_range,
            date_picker,
            selected: None,
            wanted_id: saved.selected_id.clone().map(Into::into),
            detail: DetailState::default(),
            conversation_seq: 0,
            conversation_pattern: None,
            detail_timing: None,
            detail_visible: saved.detail_visible,
            last_export_dir: None,
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
            #[cfg(test)]
            selection_event_count: None,
        };
        // a fresh window has no focus, and without one no key binding or menu action reaches the app
        this.focus_table(window, cx);
        if std::env::var_os("ASM_FAVORITES_AUTOPLAY").is_some() {
            this._autoplay = Some(this.favorites_autoplay(window, cx));
        } else if std::env::var_os("ASM_SPIKE_AUTOPLAY").is_some() {
            this._autoplay = Some(this.autoplay(window, cx));
        } else if std::env::var_os("ASM_SEARCH_AUTOPLAY").is_some() {
            this._autoplay = Some(this.search_autoplay(window, cx));
        } else if std::env::var_os("ASM_DETAIL_AUTOPLAY").is_some() {
            this._autoplay = Some(this.detail_autoplay(window, cx));
        }
        this
    }

    /// Apply a batch of store events with a single table recompute.
    fn on_store_events(&mut self, batch: Vec<StoreEvent>, window: &mut Window, cx: &mut Context<Self>) {
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
                StoreEvent::Conversation { seq, session_id, conversation, read, build } => {
                    if seq == self.conversation_seq && self.selected.as_ref().is_some_and(|row| row.id == session_id) {
                        self.show_conversation(conversation, read, build);
                    }
                }
                StoreEvent::Exported { dest, result } => self.exported(&dest, result, window, cx),
                StoreEvent::FavoriteChanged { id, favorite } => {
                    use gpui_kit::component::WindowExt;
                    window.push_notification(format!("{} {}", if favorite { "已收藏" } else { "已取消收藏" }, id), cx);
                }
                StoreEvent::FavoritesDirChanged { dir, moved, skipped } => {
                    use gpui_kit::component::WindowExt;
                    self.favorites.root = dir;
                    window.push_notification(format!("收藏目录已更改：迁移 {moved}，跳过 {skipped}"), cx);
                }
                StoreEvent::RebuildingRoots => {
                    use gpui_kit::component::WindowExt;
                    window.push_notification("数据目录已更改，正在重建索引", cx);
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
                self.open_conversation(&row);
            }
            // a few stats; the row may also have changed (favorited, the transcript cleaned up)
            self.detail.actions = actions::check(&row, &self.claude);
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
        self.detail_timing = None;
    }

    /// A session was selected: nothing of the last one stays in the detail panel while the
    /// reader thread reads its transcript, or the prompts of a history-only session.
    fn open_conversation(&mut self, row: &SessionRow) {
        self.detail.begin_loading();
        self.detail_timing = Some(DetailTiming { short_id: row.short_id.clone(), started: Instant::now(), loaded: None });
        self.read_conversation(row);
    }

    /// Ask the reader thread for `row`'s conversation, marked for the current search: its
    /// transcript, or once Claude Code cleaned that up, its prompts in `history.jsonl`.
    pub(crate) fn read_conversation(&mut self, row: &SessionRow) {
        self.conversation_pattern = self.matcher.as_ref().map(|m| m.pattern().to_string());
        let (id, matcher) = (row.id.to_string(), self.matcher.clone());
        self.conversation_seq = match row.transcript_path.clone() {
            Some(path) => self.store.read_conversation(id, path, row.project_path.to_string(), matcher),
            None => self.store.read_history(id, row.project_key.to_string(), matcher),
        };
    }

    /// The selected session's conversation arrived. Read again for another search, it keeps
    /// what the user opened.
    fn show_conversation(&mut self, conversation: Result<Arc<ConversationView>, String>, read: Duration, build: Duration) {
        match conversation {
            Ok(view) => {
                if let Some(timing) = self.detail_timing.as_mut().filter(|t| t.loaded.is_none()) {
                    timing.loaded = Some((view.bytes, read, build));
                }
                let keep_open = self.detail.has_conversation();
                self.detail.set_conversation(view, keep_open);
            }
            Err(message) => {
                eprintln!("detail: {message}");
                self.detail_timing = None;
                self.detail.fail(message);
            }
        }
    }

    /// Called while the first frame showing a newly selected conversation paints:
    /// `detail: 5f6eea99 66.7 MB read 18.2ms build 0.3ms first frame 41.0ms`.
    pub(crate) fn report_first_frame(&mut self) {
        let Some(DetailTiming { short_id, started, loaded: Some((bytes, read, build)) }) = self.detail_timing.take_if(|t| t.loaded.is_some()) else { return };
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        println!("detail: {short_id} {} read {:.1}ms build {:.1}ms first frame {:.1}ms", human_size(bytes), ms(read), ms(build), ms(started.elapsed()));
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
            custom_range: self.custom_range,
            show_empty: delegate.filter().show_empty,
            kinds: delegate.filter().kinds,
            favorites_only: delegate.filter().favorites_only,
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
        // Opt-in test fuse: let a feedback loop finish so the test can fail instead of hanging
        // GPUI's effect flush. Production builds have neither the counter nor the cutoff.
        #[cfg(test)]
        if matches!(ev, TableEvent::SelectRow(_)) && let Some(count) = self.selection_event_count.as_mut() {
            *count += 1;
            if *count > 32 {
                return;
            }
        }
        match ev {
            // Selection events are deferred notifications, not commands. Several selections
            // can queue before effects flush; replaying an old one would emit another event
            // and make two selections chase each other forever. A real click updates the
            // table first, so its current notification still synchronizes the detail/state.
            TableEvent::SelectRow(ix) if self.table.read(cx).selected_row() == Some(*ix) => self.select_row(*ix, cx),
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
                    table.delegate_mut().set_sort("cost", ColumnSort::Descending);
                    table.refresh(cx);
                    table.scroll_to_row(0, cx);
                });
                this.select_row(0, cx);
                this.set_detail_tab(DetailTab::Metadata, cx);
                this.status = format!("autoplay: {name} · 30 days · sorted by Cost desc").into();
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

    /// Scripted tour of the Conversation tab for screenshots (`make screenshot-detail`). Once the
    /// start-up scan is over, every step waits for the detail panel (and a running search) to
    /// settle, prints `autoplay-detail: <step>` (or `… skipped`) and holds the view for a capture.
    /// `ASM_AUTOPLAY_SESSION` (id prefix; default: the listed session with the most tool calls),
    /// `ASM_AUTOPLAY_REGEX` (default `cargo (test|build)`) and `ASM_AUTOPLAY_SPECIAL` (a session
    /// with compactions, AUTO cards, interruptions; the step is skipped without it) steer it;
    /// `ASM_AUTOPLAY_STEP` picks what `step-open` opens instead of a failed tool call:
    /// `cut-output` (a tool output core cut, shown whole; also `ASM_AUTOPLAY_CUT_OUTPUT=1`),
    /// `thinking`, or `mcp` (an MCP tool call). After the 70MB session: a history-only session's
    /// prompt cards, a reply with a code block (in `ASM_AUTOPLAY_SESSION`'s session again),
    /// `]` pressed twice from the top, and that session's Metadata (the files it changed).
    fn detail_autoplay(&self, window: &Window, cx: &mut Context<Self>) -> Task<()> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let session = var("ASM_AUTOPLAY_SESSION");
        let code_session = session.clone();
        let regex = var("ASM_AUTOPLAY_REGEX").unwrap_or_else(|| "cargo (test|build)".into());
        let special = var("ASM_AUTOPLAY_SPECIAL");
        // false: nothing to do, the step is skipped
        type Step = Box<dyn FnOnce(&mut AppRoot, &mut Window, &mut Context<AppRoot>) -> bool>;
        let steps: Vec<(&str, Step)> = vec![
            ("conversation", Box::new(move |this, _, cx| this.autoplay_select(session.as_deref(), cx))),
            ("steps-open", Box::new(|this, _, cx| this.autoplay_open_longest_turn(cx))),
            ("step-open", Box::new(|this, _, cx| this.autoplay_open_a_step(cx))),
            (
                "expand-all",
                Box::new(|this, _, cx| {
                    this.detail.expand_all(true);
                    this.detail.scroll_to_card(0);
                    cx.notify();
                    true
                }),
            ),
            (
                "search-hit",
                Box::new(move |this, window, cx| {
                    if this.search_opts.mode != SearchMode::Regex {
                        this.toggle_regex(cx);
                    }
                    this.set_query(&regex, window, cx);
                    true
                }),
            ),
            (
                "next-hit",
                Box::new(|this, _, cx| {
                    this.step_hit(true, cx);
                    true
                }),
            ),
            (
                "special",
                Box::new(move |this, window, cx| {
                    let Some(prefix) = special else { return false };
                    this.clear_search(window, cx);
                    this.autoplay_select(Some(&prefix), cx)
                }),
            ),
            (
                "big-session",
                Box::new(|this, window, cx| {
                    this.clear_search(window, cx);
                    this.autoplay_select(Some("5f6eea99"), cx)
                }),
            ),
            ("history-session", Box::new(|this, _, cx| this.autoplay_select_history(cx))),
            ("code-block", Box::new(move |this, _, cx| this.autoplay_select(code_session.as_deref(), cx))),
            (
                "next-turn",
                Box::new(|this, window, cx| {
                    // through the key binding's action, from the table that has the focus
                    this.detail.list.scroll_to(ListOffset::default());
                    window.dispatch_action(Box::new(NextTurn), cx);
                    window.dispatch_action(Box::new(NextTurn), cx);
                    this.status = "autoplay: ] twice from the top".into();
                    true
                }),
            ),
            ("metadata", Box::new(|this, _, cx| {
                this.set_detail_tab(DetailTab::Metadata, cx);
                true
            })),
        ];
        cx.spawn_in(window, async move |this, cx| {
            // sessions are picked from the table: let the start-up scan fill it first
            for _ in 0..1200 {
                let scanning = this.update(cx, |this, _| matches!(this.index_status, IndexStatus::Loading | IndexStatus::Scanning { .. }));
                if !scanning.unwrap_or(false) {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(100)).await;
            }
            cx.background_executor().timer(Duration::from_millis(500)).await;
            for (name, step) in steps {
                let Ok(ran) = this.update_in(cx, |this, window, cx| step(this, window, cx)) else { return };
                if !ran {
                    println!("autoplay-detail: {name} skipped");
                    continue;
                }
                // past the search debounce, then until the search and the reads it causes are done
                cx.background_executor().timer(Duration::from_millis(400)).await;
                for _ in 0..100 {
                    if this.update(cx, |this, _| this.detail_settled()).unwrap_or(true) {
                        break;
                    }
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                }
                this.update(cx, |this, cx| {
                    match name {
                        // what the stats line says while the mouse is on a tick
                        "conversation" => this.autoplay_hover_a_tick(),
                        // the cut line is at the end of the output
                        // a cut output's cut line and a thinking's last line are at the step's end
                        "step-open" if matches!(autoplay_step().as_deref(), Some("cut-output" | "thinking")) => this.autoplay_show_the_end_of_the_pointed_step(),
                        "special" => this.autoplay_scroll_to_special(),
                        // every turn is there, however long the session: show its very end
                        "big-session" => this.detail.list.scroll_to_end(),
                        "code-block" => this.autoplay_show_a_code_block(),
                        _ => {}
                    }
                    cx.notify();
                })
                .ok();
                cx.background_executor().timer(Duration::from_millis(500)).await;
                println!("autoplay-detail: {name}");
                cx.background_executor().timer(Duration::from_millis(1500)).await;
            }
        })
    }

    /// No search or conversation read is under way.
    fn detail_settled(&self) -> bool {
        let searching = self.searching() && self.search_summary.is_none() && self.search_error.is_none();
        let reading = match self.detail.load {
            Load::Loading => true,
            Load::Ready => self.detail.pattern() != self.conversation_pattern.as_deref(),
            Load::Idle | Load::Failed(_) => false,
        };
        !searching && !reading
    }

    /// Put the mouse, as far as the detail state knows, on a tick of the first turn with steps:
    /// its first failed tool call, else its middle step.
    fn autoplay_hover_a_tick(&mut self) {
        let Some(view) = self.detail.view.clone() else { return };
        let turn = view.cards.iter().enumerate().find_map(|(ix, c)| match c {
            Card::Turn(t) if !t.steps.is_empty() => Some((ix, t)),
            _ => None,
        });
        if let Some((card, turn)) = turn {
            let step = turn.steps.iter().position(|s| s.error).unwrap_or(turn.steps.len() / 2);
            self.detail.hover_tick = Some((card, step));
        }
    }

    /// Scroll the bottom of the step opened from its tick into view (it is laid out by now).
    fn autoplay_show_the_end_of_the_pointed_step(&mut self) {
        let Some((card, step)) = self.detail.pointed else { return };
        let Some(ix) = self.detail.rows.iter().position(|row| *row == crate::conversation::Row::Step(card, step)) else { return };
        if let Some(item) = self.detail.list.bounds_for_item(ix) {
            self.detail.list.scroll_by(item.bottom() - self.detail.list.viewport_bounds().bottom() + px(16.));
        }
    }

    /// Scroll to the card before the first interrupted or failed one (else the first
    /// compaction), so the special cards share the screen.
    fn autoplay_scroll_to_special(&mut self) {
        let Some(view) = self.detail.view.clone() else { return };
        let failed = view.cards.iter().position(|c| matches!(c, Card::Turn(t) if t.status == crate::conversation::Status::Failed));
        let special = failed.or_else(|| view.cards.iter().position(|c| matches!(c, Card::Compact(_))));
        if let Some(card) = special {
            self.detail.scroll_to_card(card.saturating_sub(1));
        }
    }

    /// Select the listed session whose id starts with `prefix`, or the one with the most tool
    /// calls; scrolled into view.
    fn autoplay_select(&mut self, prefix: Option<&str>, cx: &mut Context<Self>) -> bool {
        let delegate = self.table.read(cx).delegate();
        let listed = 0..delegate.ix.len();
        let target = match prefix {
            Some(prefix) => listed.clone().find(|ix| delegate.row(*ix).id.starts_with(prefix)),
            None => listed.max_by_key(|ix| delegate.row(*ix).tools),
        };
        let Some(target) = target else { return false };
        self.table.update(cx, |table, cx| table.scroll_to_row(target, cx));
        self.select_row(target, cx);
        true
    }

    /// Select the first listed history-only session with three prompts or more (else the first
    /// one), scrolled into view.
    fn autoplay_select_history(&mut self, cx: &mut Context<Self>) -> bool {
        let delegate = self.table.read(cx).delegate();
        let history = || (0..delegate.ix.len()).filter(|ix| delegate.row(*ix).kind == Kind::History);
        let Some(target) = history().find(|ix| delegate.row(*ix).prompts >= 3).or_else(|| history().next()) else { return false };
        self.table.update(cx, |table, cx| table.scroll_to_row(target, cx));
        self.select_row(target, cx);
        true
    }

    /// Show the reply with a code block nearest its start: unfolded, its card's tail at the top.
    fn autoplay_show_a_code_block(&mut self) {
        use crate::conversation::{Row, Segment, Tail};
        let Some(view) = self.detail.view.clone() else { return };
        let replies = view.cards.iter().enumerate().filter_map(|(ix, card)| {
            let Card::Turn(turn) = card else { return None };
            let Tail::Reply { text, .. } = &turn.tail else { return None };
            let code = text.segments.iter().find_map(|s| if let Segment::Code { range } = s { Some(range.start) } else { None })?;
            Some((ix, text.fold.is_some(), text.text[..code].lines().count()))
        });
        let Some((card, folded, _)) = replies.min_by_key(|(_, _, lines)| *lines) else {
            self.status = "autoplay: no reply with a code block in this session".into();
            return;
        };
        if folded && !self.detail.unfolded.contains(&(card, agent_session_core::model::Part::Reply)) {
            self.detail.toggle_fold(card, agent_session_core::model::Part::Reply);
        }
        self.detail.scroll_to(Row::Tail(card));
        self.status = format!("autoplay: the reply of card {card}, with a code block").into();
    }

    /// Open the turn with the most steps and scroll to it.
    fn autoplay_open_longest_turn(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.detail.view.clone() else { return false };
        let longest = view.cards.iter().enumerate().filter_map(|(ix, c)| if let Card::Turn(t) = c { Some((ix, t.steps.len())) } else { None }).max_by_key(|(_, steps)| *steps);
        let Some((card, steps)) = longest.filter(|(_, steps)| *steps > 0) else { return false };
        if !self.detail.open_turns.contains(&card) {
            self.detail.toggle_turn(card);
        }
        self.detail.scroll_to_card(card);
        self.status = format!("autoplay: turn card {card} with {steps} steps").into();
        cx.notify();
        true
    }

    /// In the open turn, open its first failed tool call (or else its first tool call) the way a
    /// click on its tick does; `ASM_AUTOPLAY_STEP` picks another kind of step (see
    /// `detail_autoplay`).
    fn autoplay_open_a_step(&mut self, cx: &mut Context<Self>) -> bool {
        use crate::conversation::{StepBody, TickKind};
        let (Some(view), Some(&card)) = (self.detail.view.clone(), self.detail.open_turns.iter().next()) else { return false };
        let Some(Card::Turn(turn)) = view.cards.get(card) else { return false };
        let steps = || turn.steps.iter().enumerate();
        let tools = || steps().filter(|(_, s)| s.kind == TickKind::Tool);
        let pick = autoplay_step();
        let step = match pick.as_deref() {
            Some("cut-output") => tools().find(|(_, s)| matches!(&s.body, StepBody::Tool { output: Some(out), .. } if !out.cut.is_empty())),
            Some("thinking") => steps().find(|(_, s)| s.kind == TickKind::Thinking),
            Some("mcp") => tools().find(|(_, s)| !s.full_name.is_empty()),
            _ => tools().find(|(_, s)| s.error).or_else(|| tools().next()),
        };
        let Some((step, _)) = step else { return false };
        self.detail.point_at(card, step);
        if pick.as_deref() == Some("cut-output") {
            self.detail.toggle_fold(card, agent_session_core::model::Part::Step(step));
        }
        cx.notify();
        true
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
    pub(crate) fn focus_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let (shown, total, cost) = (delegate.ix.len(), delegate.rows.len(), delegate.shown_cost);
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
            .left(format!("{shown} / {total} sessions · {}", fmt_usd(cost)))
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
            .on_action(cx.listener(|this, _: &ToggleFavorite, window, cx| this.toggle_favorite(window, cx)))
            .on_action(cx.listener(|this, action: &favorites::FavoriteSession, window, cx| this.favorite_session(&action.id, window, cx)))
            .on_action(cx.listener(|this, _: &RevealFavoritesFolder, _, cx| cx.reveal_path(&this.favorites.root)))
            .on_action(cx.listener(|this, _: &ChangeFavoritesFolder, window, cx| this.change_favorites_folder(window, cx)))
            .on_action(cx.listener(|this, _: &DefaultFavoritesFolder, window, cx| this.default_favorites_folder(window, cx)))
            .on_action(cx.listener(|this, _: &Rescan, _, cx| this.rescan(false, cx)))
            .on_action(cx.listener(|this, _: &RebuildIndex, _, cx| this.rescan(true, cx)))
            .on_action(cx.listener(|this, _: &ToggleEmptySessions, _, cx| this.toggle_empty_sessions(cx)))
            .on_action(cx.listener(|this, action: &HideProject, _, cx| this.hide_project_of(action.id.as_deref(), cx)))
            .on_action(cx.listener(|this, action: &CopySessionId, window, cx| this.copy_session_id(action.id.as_deref(), window, cx)))
            .on_action(cx.listener(|this, action: &CopyResume, window, cx| this.copy_resume(action.id.as_deref(), window, cx)))
            .on_action(cx.listener(|this, action: &RevealTranscript, window, cx| this.reveal_transcript(action.id.as_deref(), window, cx)))
            .on_action(cx.listener(|this, action: &OpenProjectFolder, window, cx| this.open_project_folder(action.id.as_deref(), window, cx)))
            .on_action(cx.listener(|this, action: &ExportTranscript, window, cx| this.export_transcript(action.id.as_deref(), window, cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_action(cx.listener(|this, _: &Escape, window, cx| this.clear_search(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleRegex, _, cx| this.toggle_regex(cx)))
            .on_action(cx.listener(|this, _: &ToggleCase, _, cx| this.toggle_case(cx)))
            .on_action(cx.listener(|this, _: &NextHit, _, cx| this.step_hit(true, cx)))
            .on_action(cx.listener(|this, _: &PrevHit, _, cx| this.step_hit(false, cx)))
            .on_action(cx.listener(|this, _: &NextTurn, _, cx| this.step_turn(true, cx)))
            .on_action(cx.listener(|this, _: &PrevTurn, _, cx| this.step_turn(false, cx)))
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

// Only the icons the component bundle lacks (the detail header's buttons); see gpui-kit-assets'
// README, "Default component icons and application extras".
gpui_kit::assets::icon_assets!(ExtraIcons, [SquareTerminal, FileSearch, Download]);

/// The default component icons plus `ExtraIcons`.
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        match ExtraIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// `ASM_AUTOPLAY_STEP`, or `cut-output` for the older `ASM_AUTOPLAY_CUT_OUTPUT=1`.
fn autoplay_step() -> Option<String> {
    std::env::var("ASM_AUTOPLAY_STEP").ok().filter(|v| !v.is_empty()).or_else(|| std::env::var_os("ASM_AUTOPLAY_CUT_OUTPUT").map(|_| "cut-output".into()))
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-d", ToggleFavorite, None),
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
        KeyBinding::new("]", NextTurn, Some("DataTable")),
        KeyBinding::new("[", PrevTurn, Some("DataTable")),
        // in the search box cmd-c copies text as usual
        KeyBinding::new("cmd-c", CopySessionId { id: None }, Some("DataTable")),
        KeyBinding::new("cmd-shift-c", CopyResume { id: None }, Some("DataTable")),
        KeyBinding::new("cmd-1", Preview1, None),
        KeyBinding::new("cmd-2", Preview2, None),
        KeyBinding::new("cmd-3", Preview3, None),
        KeyBinding::new("cmd-4", Preview4, None),
        KeyBinding::new("cmd-shift-r", RebuildIndex, None),
    ]);
}

fn main() {
    let app = gpui_kit::application().with_assets(AppAssets);
    app.run(move |cx| {
        gpui_kit::init(cx);
        style::install_theme(cx);
        cx.activate(true);
        cx.on_action(|_: &Quit, cx| cx.quit());
        bind_keys(cx);
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
                MenuItem::action("Hide Project of Selected Session", HideProject { id: None }),
            ]),
            Menu::new("Favorites").items(vec![
                MenuItem::action("Toggle Favorite", ToggleFavorite),
                MenuItem::action("Reveal Favorites Folder", RevealFavoritesFolder),
                MenuItem::action("Change Favorites Folder…", ChangeFavoritesFolder),
                MenuItem::action("Use Default Favorites Folder", DefaultFavoritesFolder),
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
                let view = cx.new(|cx| AppRoot::new(AppPaths::from_env(), window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
