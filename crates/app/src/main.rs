//! Agent Session Manager — macOS desktop app listing Claude Code sessions.
//!
//! Layout follows the Claude Design mockup: toolbar (dropdown filters + search), session table,
//! detail panel with Metadata / Conversation tabs, status bar. Sessions come from `crates/core`
//! through `store` (index load, then a streaming incremental scan). Set `ASM_SPIKE_AUTOPLAY=1`
//! to scroll / select / filter / sort automatically after launch (used by `make screenshot`),
//! `ASM_SEARCH_AUTOPLAY=1` to run a scripted search session (`make screenshot-search`),
//! `ASM_DETAIL_AUTOPLAY=1` to tour the Conversation tab (`make screenshot-detail`),
//! `ASM_COST_AUTOPLAY=1` to tour the Cost tab (`make screenshot-cost`),
//! `ASM_USAGE_AUTOPLAY=<dir>` to scan and rebuild when files show up in `<dir>` (`make screenshot-usage`),
//! `ASM_RESIDENT_AUTOPLAY=<dir>` to close, reopen and work the menu bar item's menu the same way
//! (`make screenshot-resident`), `ASM_USAGE_VIEW_AUTOPLAY=<dir>` to tour the Usage view, one step
//! per capture (`make screenshot-usage-view`);
//! `ASM_CLAUDE_DIR`, `ASM_DB` and `ASM_STATE` point the app at other data.
//!
//! While the app is open it scans by itself (`auto_scan`): when its window comes to the front and
//! every `AUTO_SCAN_IDLE`, at most once per `AUTO_SCAN_MIN_GAP`. The status bar shows what today
//! (from 4:00 local) cost, from the usage ledger.
//!
//! Closing the window only hides it (`close_requested`): the app stays in the menu bar
//! (`resident`), scanning as before, until it is quit. Open Sessions in the menu bar item's menu,
//! or opening the app again from Finder, shows the same window again (`show_window`).

mod actions;
mod conversation;
mod cost_tab;
mod detail;
mod drilldown;
mod favorites;
#[cfg(test)]
mod keyboard_tests;
mod model;
mod resident;
mod search;
mod settings;
mod store;
mod style;
mod table;
mod toolbar;
mod turn_cards;
mod updater;
mod usage_chart;
mod usage_view;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_session_core::claude::paths::{ClaudeDir, default_db_path};
use agent_session_core::claude::model_catalog::ModelCatalog;
use agent_session_core::format::{fmt_change, fmt_usd, human_size};
use agent_session_core::pricing::display_name;
use agent_session_core::index::ScanReport;
use agent_session_core::search::Matcher;
use agent_session_core::usage::{Totals, UsageData, UsageRange, day_start_ms};

use chrono::{Local, NaiveDate};
use gpui_kit::component::date_picker::{DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{Escape, InputEvent, InputState};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::table::{ColumnSort, DataTable, TableEvent, TableState};
use gpui_kit::component::{ResizableState, Root, Sizable, h_flex, h_resizable, resizable_panel, v_flex};
use gpui_kit::*;

use crate::actions::{CopyResume, CopySessionId, ExportTranscript, HideProject, OpenProjectFolder, RevealTranscript};
use crate::conversation::{Card, ConversationView};
use crate::detail::{DETAIL_W, DetailState, DetailTab, Load};
use crate::model::{DisplayMode, Filter, Kind, MainView, ModelEntry, ProjectEntry, RangePreset, SessionRow, kind_counts, model_entries, project_entries, split_hidden, today_at};
use crate::resident::{Resident, ResidentCommand, ResidentView, ScanRecord};
use crate::search::{RowHit, SearchMode, SearchOptions, SearchSummary, Searcher};
use crate::settings::{SortDir, ViewState, default_state_path};
use crate::store::{Store, StoreEvent, StoreRequest};
use crate::table::{PREVIEW_MIN_W, ROW_H, SessionTableDelegate, ShowAllColumns, ToggleColumn};
use crate::updater::{About, CheckForUpdates};
use crate::style::FluentBuilder;
use crate::usage_view::UsageState;

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
        ToggleEmptySessions,
        CloseWindow,
        ShowSessions,
        ShowUsage,
        LeaveDay
    ]
);

const WINDOW_W: f32 = 1440.;
const WINDOW_H: f32 = 900.;
/// Space the table needs besides its columns: trailing empty column + vertical scrollbar.
const TABLE_CHROME_W: f32 = 16.;
/// An automatic scan is skipped while one runs or within this long after the last one ended.
const AUTO_SCAN_MIN_GAP: Duration = Duration::from_secs(30);
/// While the app is open and nothing else scanned, it scans this often.
const AUTO_SCAN_IDLE: Duration = Duration::from_secs(600);
/// … and this often while the Usage view is on screen.
const USAGE_SCAN_IDLE: Duration = Duration::from_secs(60);
/// The status bar when a scan cannot start because the store thread is gone (and no earlier
/// error is on it).
const STORE_GONE: &str = "cannot scan: the index is not open, restart the app";

/// The first close of the window says what closing does now (`close_requested`).
const RESIDENT_NOTICE: &str = "Agent Session Manager 会留在菜单栏继续记录用量。";
const RESIDENT_NOTICE_DETAIL: &str = "要完全退出请按 ⌘Q。";
const RESIDENT_NOTICE_ANSWERS: [&str; 2] = ["好", "立即退出"];

/// What the index is doing, shown in the middle of the status bar.
#[derive(Clone, Debug, PartialEq)]
enum IndexStatus {
    Loading,
    Scanning { done: usize, total: usize },
    Idle(SharedString),
    Failed(SharedString),
}

/// Why the search has to run again once the session table is back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum StaleSearch {
    /// A scan reloaded the table: the same search, swapped in silently (`refresh_search`).
    Refresh,
    /// The filters changed: the sessions searched are others (`run_search`).
    Rerun,
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
    catalog: Arc<ModelCatalog>,
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
    /// Main models of the listed sessions, for the Model dropdown.
    models: Vec<ModelEntry>,
    /// What the Cost tab prices the selected session as (a model with a price).
    compare_model: SharedString,
    searcher: Searcher,
    search_opts: SearchOptions,
    /// The query being searched for (trimmed); empty when no search is on.
    query: SharedString,
    /// Marks the hits of `query` in the detail panel.
    matcher: Option<Matcher>,
    /// Events of any other search are stale.
    search_seq: u64,
    /// The table still shows what came before `search_seq`; its first event swaps the results
    /// (a refresh: its end does).
    search_pending: bool,
    /// The running search repeats the one on screen after a reload (`refresh_search`): its hits
    /// gather here and replace the results in one go once it is done. `None`: a new search.
    search_refresh: Option<Vec<(SharedString, Arc<RowHit>)>>,
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
    /// Hourly usage from the ledger: every call the app ever read, by session.
    usage: UsageData,
    /// `None` until the store says whether this run has a ledger.
    usage_available: Option<bool>,
    /// The usage day (from 4:00 local) the range presets, the date picker and `today` are on;
    /// `roll_day` moves it on.
    usage_day: NaiveDate,
    /// All calls since 4:00 of `usage_day`, whatever the table shows.
    today: Totals,
    /// `Today $1.23 (2 unpriced)`, `Today —` without a ledger; empty until the ledger is read.
    today_label: SharedString,
    /// A scan was asked for (or the start-up one runs) and has not ended.
    scan_in_flight: bool,
    /// When the last scan ended (executor clock, so tests can move it).
    last_scan_done: Option<Instant>,
    /// Scans that reached `ScanDone`.
    scans_done: usize,
    /// When scans last went through or failed, for the menu bar item's menu.
    scan_record: ScanRecord,
    /// The window was closed: it is off screen and the app lives in the menu bar.
    window_hidden: bool,
    /// The first close explained that the app stays in the menu bar (saved in `state.json`).
    resident_notice_shown: bool,
    /// That explanation is on screen, waiting for an answer.
    notice_open: bool,
    /// Quit was asked for (the test platform's quit does nothing, so tests look here).
    quitting: bool,
    /// What the menu bar item shows, last pushed to it by `sync_resident`.
    resident_view: Option<ResidentView>,
    /// The session table or the Usage view.
    view: MainView,
    usage_view: UsageState,
    /// The search results are out of date and were not searched again while Usage was shown:
    /// that happens on the way back to the session table (`set_view`).
    search_stale: Option<StaleSearch>,
    launched: Instant,
    first_rows_ms: Option<u128>,
    state_path: PathBuf,
    saved: ViewState,
    _subscriptions: Vec<Subscription>,
    _events: Task<()>,
    _search_events: Task<()>,
    _debounce: Option<Task<()>>,
    _autoplay: Option<Task<()>>,
    _auto_scan: Task<()>,
    _menu_commands: Option<Task<()>>,
    #[cfg(test)]
    selection_event_count: Option<usize>,
    /// Every notification `say` pushed, in order.
    #[cfg(test)]
    notices: Vec<SharedString>,
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

/// No transcript was new, changed or gone, and history.jsonl brought nothing.
fn scan_changed_nothing(report: &ScanReport) -> bool {
    report.files_parsed + report.files_appended + report.files_rebuilt == 0 && report.files_missing == 0 && report.history_sessions == 0
}

fn scan_summary(report: &ScanReport) -> SharedString {
    let changed = report.files_parsed + report.files_appended + report.files_rebuilt;
    let secs = report.elapsed.as_secs_f64();
    if scan_changed_nothing(report) {
        format!("index up to date · {} files checked in {secs:.2}s", report.files_seen).into()
    } else {
        format!("indexed {changed} of {} files in {secs:.2}s", report.files_seen).into()
    }
}

impl AppRoot {
    fn new(paths: AppPaths, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state_path = paths.state;
        let saved = ViewState::load(&state_path);
        let usage_day = today_at(now_ms());
        let mut delegate = SessionTableDelegate::new(paths.claude.clone());
        delegate.mode = saved.mode;
        let (since_ms, until_ms) = saved.range.bounds_on(usage_day, saved.custom_range);
        delegate.set_filter(Filter {
            project: saved.project_key.clone().map(Into::into),
            since_ms,
            until_ms,
            show_empty: saved.show_empty,
            kinds: saved.kinds,
            model: saved.model.clone().map(Into::into),
            favorites_only: saved.favorites_only,
            hidden_projects: saved.hidden_projects.iter().cloned().map(Into::into).collect(),
            // a jump from Usage is not kept across a restart: its range came from that moment
            active: None,
        });
        // before the sort: a hidden column is not sorted by
        delegate.set_hidden_columns(saved.hidden_columns.iter().cloned());
        if let Some((key, dir)) = &saved.sort {
            delegate.set_sort(key, dir.to_column());
        }
        // a selected column only hides the selected row; header clicks are for sorting
        let table = cx.new(|cx| TableState::new(delegate, window, cx).col_selectable(false));

        // escape is handled by `clear_search` (the box propagates it), which also hands the focus back
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions"));
        let resizable = cx.new(|_| ResizableState::default());
        let shown_range = saved.custom_range.unwrap_or_else(|| RangePreset::default_custom(usage_day));
        let date_picker = cx.new(|cx| {
            let mut picker = DatePickerState::range(window, cx).date_format("%Y-%m-%d");
            picker.set_date(shown_range, window, cx);
            picker
        });
        let usage_shown_range = saved.usage_custom_range.unwrap_or_else(|| RangePreset::default_custom(usage_day));
        let usage_picker = cx.new(|cx| {
            let mut picker = DatePickerState::range(window, cx).date_format("%Y-%m-%d");
            picker.set_date(usage_shown_range, window, cx);
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
            cx.subscribe_in(&usage_picker, window, |this, _, ev: &DatePickerEvent, window, cx| {
                let DatePickerEvent::Change(date) = ev;
                if let (Some(start), Some(end)) = (date.start(), date.end()) {
                    this.set_usage_custom_range(start, end, cx);
                }
                this.focus_view(window, cx);
            }),
            // back to the window after working elsewhere: show what happened meanwhile
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.roll_day(now_ms(), cx);
                    this.auto_scan("window activated", cx);
                }
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

        // the red button: hide instead of close (AppKit asks first, and a false keeps the window)
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.close_requested(window, cx)).ok();
            false
        });

        let auto_scan = Self::spawn_auto_scan(cx);
        let usage_view = UsageState::new(&saved, usage_picker, cx.focus_handle());

        let mut this = Self {
            claude: paths.claude,
            catalog: Arc::new(ModelCatalog::default()),
            favorites: paths.favorites,
            table,
            search,
            resizable,
            store,
            projects: Vec::new(),
            hidden_projects: Vec::new(),
            kind_counts: [0; Kind::ALL.len()],
            models: Vec::new(),
            compare_model: saved
                .compare_model
                .clone()
                .filter(|m| agent_session_core::pricing::price(m).is_some())
                .unwrap_or_else(|| crate::cost_tab::DEFAULT_COMPARE_MODEL.to_string())
                .into(),
            searcher,
            search_opts: saved.search.clone(),
            query: "".into(),
            matcher: None,
            search_seq: 0,
            search_pending: false,
            search_refresh: None,
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
            usage: UsageData::default(),
            usage_available: None,
            usage_day,
            today: Totals::default(),
            today_label: "".into(),
            // the store thread scans right after it opened the index
            scan_in_flight: true,
            last_scan_done: None,
            scans_done: 0,
            scan_record: ScanRecord::default(),
            window_hidden: false,
            resident_notice_shown: saved.resident_notice_shown,
            notice_open: false,
            quitting: false,
            resident_view: None,
            view: saved.view,
            usage_view,
            search_stale: None,
            launched: Instant::now(),
            first_rows_ms: None,
            state_path,
            saved,
            _subscriptions: subscriptions,
            _events: pump,
            _search_events: search_pump,
            _debounce: None,
            _autoplay: None,
            _auto_scan: auto_scan,
            _menu_commands: None,
            #[cfg(test)]
            selection_event_count: None,
            #[cfg(test)]
            notices: Vec::new(),
        };
        // a fresh window has no focus, and without one no key binding or menu action reaches the app
        this.focus_view(window, cx);
        this.sync_resident(cx);
        if std::env::var_os("ASM_FAVORITES_AUTOPLAY").is_some() {
            this._autoplay = Some(this.favorites_autoplay(window, cx));
        } else if std::env::var_os("ASM_SPIKE_AUTOPLAY").is_some() {
            this._autoplay = Some(this.autoplay(window, cx));
        } else if std::env::var_os("ASM_SEARCH_AUTOPLAY").is_some() {
            this._autoplay = Some(this.search_autoplay(window, cx));
        } else if std::env::var_os("ASM_DETAIL_AUTOPLAY").is_some() {
            this._autoplay = Some(this.detail_autoplay(window, cx));
        } else if std::env::var_os("ASM_COST_AUTOPLAY").is_some() {
            this._autoplay = Some(this.cost_autoplay(window, cx));
        } else if let Some(gate) = std::env::var_os("ASM_USAGE_AUTOPLAY").filter(|v| !v.is_empty()) {
            this._autoplay = Some(this.usage_autoplay(PathBuf::from(gate), window, cx));
        } else if let Some(gate) = std::env::var_os("ASM_RESIDENT_AUTOPLAY").filter(|v| !v.is_empty()) {
            this._autoplay = Some(this.resident_autoplay(PathBuf::from(gate), window, cx));
        } else if let Some(gate) = std::env::var_os("ASM_USAGE_VIEW_AUTOPLAY").filter(|v| !v.is_empty()) {
            this._autoplay = Some(this.usage_view_autoplay(PathBuf::from(gate), window, cx));
        }
        this
    }

    /// Apply a batch of store events with a single table recompute.
    fn on_store_events(&mut self, batch: Vec<StoreEvent>, window: &mut Window, cx: &mut Context<Self>) {
        let mut streamed = Vec::new();
        let mut data_changed = false;
        let mut reloaded = false;
        let mut usage_changed = false;
        for ev in batch {
            match ev {
                StoreEvent::Loaded { sessions, catalog, elapsed } => {
                    self.catalog = catalog;
                    let rows: Vec<Arc<SessionRow>> = sessions.iter().map(|s| Arc::new(SessionRow::from_summary(s, &self.catalog))).collect();
                    streamed.clear();
                    if !rows.is_empty() && self.first_rows_ms.is_none() {
                        self.first_rows_ms = Some(self.launched.elapsed().as_millis());
                        println!("{} sessions on screen {}ms after launch (index read {elapsed:?})", rows.len(), self.launched.elapsed().as_millis());
                    }
                    self.table.update(cx, |table, _| table.delegate_mut().set_rows(rows));
                    data_changed = true;
                    reloaded = true;
                }
                StoreEvent::Session(s) => streamed.push(Arc::new(SessionRow::from_summary(&s, &self.catalog))),
                StoreEvent::Progress(p) if p.done < p.total => self.index_status = IndexStatus::Scanning { done: p.done, total: p.total },
                StoreEvent::Progress(_) => {}
                StoreEvent::ScanDone { report, stats, quiet } => {
                    println!("scan{}: {report:?}", if quiet { " (automatic)" } else { "" });
                    // an automatic scan that found nothing leaves the summary of the last one
                    if !(quiet && scan_changed_nothing(&report)) {
                        self.index_status = IndexStatus::Idle(scan_summary(&report));
                    }
                    self.db_info = format!("{} messages · DB {}", stats.messages, human_size(stats.db_bytes)).into();
                    self.scans_done += 1;
                    self.scan_record.done(Local::now().time());
                    self.scan_ended(cx);
                    data_changed = true;
                    usage_changed = true;
                }
                StoreEvent::ScanSkipped => {
                    self.scan_record.failed(Local::now().time());
                    self.scan_ended(cx);
                }
                StoreEvent::Usage { sessions, full } => {
                    if full {
                        self.usage.replace_all(sessions);
                        self.usage_available = Some(true);
                    } else {
                        self.usage.apply(sessions);
                    }
                    usage_changed = true;
                }
                StoreEvent::UsageUnavailable => {
                    self.usage_available = Some(false);
                    usage_changed = true;
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
                    if self.scan_in_flight {
                        self.scan_record.failed(Local::now().time());
                    }
                    self.scan_ended(cx);
                }
            }
        }
        if !streamed.is_empty() {
            self.first_rows_ms.get_or_insert(self.launched.elapsed().as_millis());
            self.table.update(cx, |table, _| table.delegate_mut().upsert(streamed));
            data_changed = true;
        }
        if data_changed {
            // the user did not ask for this refresh: the selected session keeps its place in view
            // or out of it, the table is not scrolled back to it
            self.relocate_selection(false, cx);
        }
        if usage_changed {
            self.roll_day(now_ms(), cx);
        }
        // new calls in the Active filter's days: other sessions are listed, as after a scan
        let active_changed = usage_changed && self.sync_active_ids(cx);
        if active_changed {
            self.relocate_selection(false, cx);
        }
        // a reload follows a scan that changed the index: the results on screen may be out of date
        // (not searched while Usage is shown: Usage scans every minute, a regex search takes a while)
        if (reloaded || active_changed) && self.searching() {
            match self.view {
                MainView::Sessions => self.refresh_search(cx),
                MainView::Usage => self.search_went_stale(StaleSearch::Refresh),
            }
        }
        self.sync_resident(cx);
        cx.notify();
    }

    /// Rows or the filter changed: refresh the dropdown counts and keep the selection on its session.
    pub(crate) fn after_rows_changed(&mut self, cx: &mut Context<Self>) {
        self.relocate_selection(true, cx);
    }

    /// `after_rows_changed`; `scroll`: false when a scan changed the rows, so that a selected
    /// session that only moved to another row does not pull the table back to it (a newly picked
    /// selection is still scrolled into view).
    fn relocate_selection(&mut self, scroll: bool, cx: &mut Context<Self>) {
        let delegate = self.table.read(cx).delegate();
        let (show_empty, hidden) = (delegate.filter().show_empty, &delegate.filter().hidden_projects);
        let listed = || delegate.rows.iter().filter(|r| show_empty || r.prompts > 0);
        (self.projects, self.hidden_projects) = split_hidden(project_entries(listed()), hidden);
        self.kind_counts = kind_counts(listed().filter(|r| !hidden.contains(&r.project_key)));
        self.models = model_entries(listed().filter(|r| !hidden.contains(&r.project_key)));
        let wanted = self.wanted_id.clone().or_else(|| self.selected.as_ref().map(|r| r.id.clone()));
        let position = wanted.as_ref().and_then(|id| delegate.position_of(id));
        let has_rows = !delegate.ix.is_empty();
        // While a scan streams rows in, the top row keeps changing (and a selection restored from
        // the saved state may still be on its way): pick a default only once things settle.
        let scanning = matches!(self.index_status, IndexStatus::Scanning { .. });
        let waiting = self.selected.is_none() && (scanning || (self.wanted_id.is_some() && self.index_status == IndexStatus::Loading));
        match position {
            Some(row_ix) => self.select_row_scrolling(row_ix, scroll, cx),
            None if waiting => {}
            None if has_rows => self.select_row(0, cx),
            None => self.clear_selection(cx),
        }
        self.table.update(cx, |_, cx| cx.notify());
    }

    fn select_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        self.select_row_scrolling(row_ix, true, cx);
    }

    /// `scroll`: false leaves the table's scroll position alone when the row is the session
    /// already selected (`set_selected_row` always scrolls; its scroll is a deferred request on
    /// the list's handle, taken back here before the next frame applies it).
    fn select_row_scrolling(&mut self, row_ix: usize, scroll: bool, cx: &mut Context<Self>) {
        let selected_id = self.selected.as_ref().map(|r| r.id.clone());
        let row = self.table.update(cx, |table, cx| {
            let row = table.delegate().ix.get(row_ix).map(|_| table.delegate().row(row_ix).clone());
            if row.is_some() && table.selected_row() != Some(row_ix) {
                let moved = !scroll && row.as_ref().map(|r| &r.id) == selected_id.as_ref();
                let pending = moved.then(|| table.vertical_scroll_handle.0.borrow_mut().deferred_scroll_to_item.take());
                table.set_selected_row(row_ix, cx);
                if let Some(pending) = pending {
                    table.vertical_scroll_handle.0.borrow_mut().deferred_scroll_to_item = pending;
                }
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
            model: delegate.filter().model.as_ref().map(|m| m.to_string()),
            compare_model: Some(self.compare_model.to_string()),
            favorites_only: delegate.filter().favorites_only,
            hidden_projects: delegate.filter().hidden_projects.iter().map(|p| p.to_string()).collect(),
            hidden_columns: delegate.hidden_columns(),
            detail_visible: self.detail_visible,
            selected_id: self.selected.as_ref().map(|r| r.id.to_string()).or_else(|| self.wanted_id.as_ref().map(|id| id.to_string())),
            search: self.search_opts.clone(),
            resident_notice_shown: self.resident_notice_shown,
            view: self.view,
            // a day clicked on the chart is not kept: the range it came from is
            usage_range: if self.usage_view.range == UsageRange::Day { self.usage_view.back } else { self.usage_view.range },
            usage_custom_range: self.usage_view.custom,
            usage_metric: self.usage_view.metric,
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

    /// Scan for Changes / Rebuild Index from the menu.
    fn rescan(&mut self, rebuild: bool, cx: &mut Context<Self>) {
        if self.request_scan(if rebuild { StoreRequest::Rebuild } else { StoreRequest::Scan { quiet: false } }, cx) {
            self.index_status = IndexStatus::Loading;
        }
        cx.notify();
    }

    /// Hands a scan to the store thread. That thread stops when the index cannot be opened;
    /// then the scan fails at once (the menu says `Scan failed`) instead of waiting for an end
    /// that never comes, and the status bar keeps the error that stopped it. False then.
    fn request_scan(&mut self, request: StoreRequest, cx: &mut Context<Self>) -> bool {
        self.scan_in_flight = true;
        let sent = self.store.request(request);
        if !sent {
            eprintln!("store: {STORE_GONE}");
            if !matches!(self.index_status, IndexStatus::Failed(_)) {
                self.index_status = IndexStatus::Failed(STORE_GONE.into());
            }
            self.scan_record.failed(Local::now().time());
            self.scan_ended(cx);
        }
        self.sync_resident(cx);
        sent
    }

    /// A quiet incremental scan (window activated, the idle timer), unless one runs or the last
    /// one ended less than `AUTO_SCAN_MIN_GAP` ago. Nothing on screen changes unless the scan
    /// finds something. True when the store thread took the scan.
    pub(crate) fn auto_scan(&mut self, trigger: &str, cx: &mut Context<Self>) -> bool {
        let now = cx.background_executor().now();
        if self.scan_in_flight || self.last_scan_done.is_some_and(|t| now.saturating_duration_since(t) < AUTO_SCAN_MIN_GAP) {
            return false;
        }
        println!("auto-scan: {trigger}");
        self.request_scan(StoreRequest::Scan { quiet: true }, cx)
    }

    /// A scan ended (done, skipped or failed): the next automatic one may go after the gap.
    fn scan_ended(&mut self, cx: &App) {
        self.scan_in_flight = false;
        self.last_scan_done = Some(cx.background_executor().now());
    }

    /// `now_ms` may be on a new usage day (past 4:00): move the range presets onto it, then
    /// count today again.
    pub(crate) fn roll_day(&mut self, now_ms: i64, cx: &mut Context<Self>) {
        let day = today_at(now_ms);
        if day != self.usage_day {
            println!("usage day: {} → {day}", self.usage_day);
            self.usage_day = day;
            self.set_range(self.range, cx);
        }
        self.refresh_today(now_ms);
        self.refresh_usage(cx);
        self.sync_resident(cx);
        cx.notify();
    }

    /// Today's totals and the status bar's `Today …`. The upper end is the next whole UTC hour:
    /// the ledger's hours are whole, and `now` would leave out the hour under way.
    fn refresh_today(&mut self, now_ms: i64) {
        let from = day_start_ms(self.usage_day, &Local);
        let until = (now_ms.div_euclid(3_600_000) + 1) * 3_600_000;
        self.today = self.usage.totals(from, until);
        let label: SharedString = match self.usage_available {
            None => "".into(),
            Some(false) => "Today —".into(),
            Some(true) => {
                let unpriced = match self.today.unpriced_responses {
                    0 => String::new(),
                    n => format!(" ({n} unpriced)"),
                };
                format!("Today {}{unpriced}", fmt_usd(self.today.cost.total())).into()
            }
        };
        if label != self.today_label {
            println!("today: {label} (since {} 4:00, {} calls)", self.usage_day, self.today.responses);
            self.today_label = label;
        }
    }

    /// The menu bar item's title and menu from today's totals and the scans, pushed to it when
    /// they changed (tests have no menu bar item: only `resident_view` changes).
    fn sync_resident(&mut self, cx: &mut Context<Self>) {
        let today = (self.usage_available == Some(true)).then_some(&self.today);
        let view = ResidentView::new(today, &self.scan_record, self.scan_in_flight);
        if self.resident_view.as_ref() == Some(&view) {
            return;
        }
        println!("menu bar: {} | {} | {} | {}", view.title, view.today, view.tokens, view.scanned);
        if let Some(resident) = cx.try_global::<Resident>() {
            resident.update(&view);
        }
        self.resident_view = Some(view);
    }

    /// The red button, `⌘W` or File > Close Window: hide the window, the app goes on in the menu
    /// bar. The very first time, a notice says so first, and offers to quit instead.
    pub(crate) fn close_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_hidden || self.notice_open || self.quitting {
            return;
        }
        if self.resident_notice_shown {
            self.hide_window(window, cx);
            return;
        }
        // a sheet on the window, shown after this update; the answer comes back on a task
        println!("window: first-close notice");
        self.notice_open = true;
        let answer = window.prompt(PromptLevel::Info, RESIDENT_NOTICE, Some(RESIDENT_NOTICE_DETAIL), &RESIDENT_NOTICE_ANSWERS, cx);
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await.ok();
            this.update_in(cx, |this, window, cx| {
                this.notice_open = false;
                let Some(answer) = answer else { return };
                println!("window: first-close notice · {}", RESIDENT_NOTICE_ANSWERS.get(answer).unwrap_or(&"?"));
                this.resident_notice_shown = true;
                this.persist(cx);
                match answer {
                    1 => this.quit(cx),
                    _ => this.hide_window(window, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn hide_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(resident) = cx.try_global::<Resident>() {
            resident.hide(window);
        }
        self.window_hidden = true;
        println!("window: hidden");
        // a hidden Usage view is not worth a scan a minute: back to every ten
        self._auto_scan = Self::spawn_auto_scan(cx);
        cx.notify();
    }

    /// Open Sessions in the menu bar item's menu, or the app opened again from Finder: the
    /// window as it was, in front, the Dock icon back and the keys going to the table.
    pub(crate) fn show_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(resident) = cx.try_global::<Resident>() {
            resident.show();
        }
        // GPUI's activate is activateIgnoringOtherApps: it works without a click while a
        // terminal is in front (the autoplay). A click on Open Sessions and Finder's reopen have
        // asked for the front already (`resident::bring_forward`), which is what works while a
        // game is in front
        cx.activate(true);
        window.activate_window();
        if std::mem::take(&mut self.window_hidden) {
            println!("window: shown");
        }
        self.focus_view(window, cx);
        self.roll_day(now_ms(), cx);
        self.auto_scan("window shown", cx);
        // back to every minute if Usage is on screen
        self._auto_scan = Self::spawn_auto_scan(cx);
        cx.notify();
    }

    pub(crate) fn on_resident_command(&mut self, command: ResidentCommand, window: &mut Window, cx: &mut Context<Self>) {
        println!("menu bar: {command:?}");
        match command {
            ResidentCommand::OpenSessions => {
                self.show_window(window, cx);
                self.set_view(MainView::Sessions, window, cx);
            }
            ResidentCommand::OpenUsage => {
                self.show_window(window, cx);
                self.set_view(MainView::Usage, window, cx);
            }
            ResidentCommand::Reopen => self.show_window(window, cx),
            ResidentCommand::ScanNow => self.rescan(false, cx),
            ResidentCommand::Quit => self.quit(cx),
        }
    }

    /// Run the commands of the menu bar item's menu (and Finder's reopen) as they come.
    pub(crate) fn listen_to_menu(&mut self, commands: async_channel::Receiver<ResidentCommand>, window: &mut Window, cx: &mut Context<Self>) {
        self._menu_commands = Some(cx.spawn_in(window, async move |this, cx| {
            while let Ok(command) = commands.recv().await {
                if this.update_in(cx, |this, window, cx| this.on_resident_command(command, window, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn quit(&mut self, cx: &mut Context<Self>) {
        self.quitting = true;
        cx.quit();
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
                self.sync_projection(cx);
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

    /// Scripted scans for the usage acceptance (`make screenshot-usage`). Once the start-up scan
    /// and the ledger are in, prints `autoplay-usage: ready · <Today …>`; then waits for a file
    /// named `scan` in `gate` and does Scan for Changes, then for `rebuild` and does Rebuild
    /// Index, each time printing `autoplay-usage: scanned · …` / `rebuilt · …` once the scan has
    /// ended and the table reloaded. Whatever the script does to the data goes in between.
    fn usage_autoplay(&self, gate: PathBuf, window: &Window, cx: &mut Context<Self>) -> Task<()> {
        let settled = |this: &AppRoot| !this.scan_in_flight && !this.today_label.is_empty() && matches!(this.index_status, IndexStatus::Idle(_));
        cx.spawn_in(window, async move |this, cx| {
            for _ in 0..1200 {
                if this.update(cx, |this, _| settled(this)).unwrap_or(true) {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(100)).await;
            }
            cx.background_executor().timer(Duration::from_millis(800)).await;
            let Ok(today) = this.update(cx, |this, _| this.today_label.clone()) else { return };
            println!("autoplay-usage: ready · {today}");
            for (trigger, step, rebuild) in [("scan", "scanned", false), ("rebuild", "rebuilt", true)] {
                while !gate.join(trigger).exists() {
                    cx.background_executor().timer(Duration::from_millis(200)).await;
                }
                let Ok(base) = this.update(cx, |this, cx| {
                    this.rescan(rebuild, cx);
                    this.scans_done
                }) else {
                    return;
                };
                for _ in 0..1200 {
                    if this.update(cx, |this, _| this.scans_done > base && settled(this)).unwrap_or(true) {
                        break;
                    }
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                }
                // the reload after the scan, and a frame
                cx.background_executor().timer(Duration::from_millis(800)).await;
                let Ok(today) = this.update(cx, |this, _| this.today_label.clone()) else { return };
                println!("autoplay-usage: {step} · {today}");
            }
        })
    }

    /// Scripted menu bar steps for the resident acceptance (`make screenshot-resident`). Once the
    /// start-up scan and the ledger are in, prints `autoplay-resident: shown · <menu bar>`. Then
    /// it runs the commands the script drops into `gate` as files `1-<command>`, `2-<command>`, …
    /// in that order, and prints `autoplay-resident: <command> · …` once each is through:
    /// - `close`: what the red button does (if the first-close notice shows, it waits for it to be
    ///   answered by hand);
    /// - `menu`: opens the menu bar item's menu at a fixed place, as a click on the item would
    ///   (`ASM_RESIDENT_MENU_AT=x,y`, top-left corner from the top-left of the main screen;
    ///   default near the right end of the menu bar), and prints where;
    /// - `close-menu`: closes it without choosing anything;
    /// - `scan`: Scan Now through its menu item (the menu is closed first), until the scan ended;
    /// - `wait-scan`: until a scan started since the last step ended (the script clicked Scan Now);
    /// - `open`: Open Sessions through its menu item, until the window is back;
    /// - `wait-open`: until the window is back (the script clicked Open Sessions, or opened the
    ///   app again from outside like Finder does);
    /// - `quit`: Quit through its menu item.
    ///
    /// `<menu bar>` is the item's title and the menu's three lines, joined by `|`. With
    /// `ASM_RESIDENT_QUERY`, that query is searched for before `shown`, so the screenshots show
    /// whether a search survives the window being closed and opened again.
    fn resident_autoplay(&self, gate: PathBuf, window: &Window, cx: &mut Context<Self>) -> Task<()> {
        let query = std::env::var("ASM_RESIDENT_QUERY").ok().filter(|q| !q.is_empty());
        let settled = |this: &AppRoot| !this.scan_in_flight && !this.today_label.is_empty() && matches!(this.index_status, IndexStatus::Idle(_));
        let menu_open = |cx: &App| cx.try_global::<Resident>().is_some_and(|r| r.menu_open());
        let at = std::env::var("ASM_RESIDENT_MENU_AT").ok().and_then(|v| {
            let (x, y) = v.split_once(',')?;
            Some((x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?))
        });
        cx.spawn_in(window, async move |this, cx| {
            if !autoplay_wait(&this, cx, 1200, |this, _| settled(this)).await {
                return;
            }
            if let Some(query) = query {
                this.update_in(cx, |this, window, cx| this.set_query(&query, window, cx)).ok();
                autoplay_wait(&this, cx, 100, |this, _| this.search_summary.is_some() || this.search_error.is_some()).await;
            }
            cx.background_executor().timer(Duration::from_millis(800)).await;
            let Ok(shown) = this.update(cx, |this, _| format!("{} · status bar {}", this.resident_line(), this.today_label)) else { return };
            println!("autoplay-resident: shown · {shown}");
            let mut base = this.update(cx, |this, _| this.scans_done).unwrap_or(0);
            for n in 1.. {
                let command = loop {
                    if let Some(command) = gate_command(&gate, n) {
                        break command;
                    }
                    cx.background_executor().timer(Duration::from_millis(200)).await;
                };
                let note = match command.as_str() {
                    "close" => {
                        this.update_in(cx, |this, window, cx| this.close_requested(window, cx)).ok();
                        // the notice, if any, is answered by hand
                        let hidden = autoplay_wait(&this, cx, 6000, |this, _| this.window_hidden || this.quitting).await;
                        format!("hidden={hidden}")
                    }
                    "menu" => {
                        let Ok(at) = this.update(cx, |_, cx| cx.try_global::<Resident>().map(|r| r.pop_up_later(at))) else { return };
                        let open = autoplay_wait(&this, cx, 30, |_, cx| menu_open(cx)).await;
                        match at {
                            Some((x, y)) if open => format!("open at {x:.0},{y:.0}"),
                            _ => "not open".into(),
                        }
                    }
                    "close-menu" => {
                        this.update(cx, |_, cx| cx.try_global::<Resident>().map(|r| r.close_menu())).ok();
                        let closed = autoplay_wait(&this, cx, 30, |_, cx| !menu_open(cx)).await;
                        format!("closed={closed}")
                    }
                    "scan" | "wait-scan" => {
                        if command == "scan" {
                            this.update(cx, |_, cx| cx.try_global::<Resident>().map(|r| r.close_menu())).ok();
                            autoplay_wait(&this, cx, 30, |_, cx| !menu_open(cx)).await;
                            autoplay_menu_item(&this, cx, ResidentCommand::ScanNow);
                        }
                        let done = autoplay_wait(&this, cx, 1200, |this, _| this.scans_done > base && settled(this)).await;
                        let Ok(line) = this.update(cx, |this, _| this.resident_line()) else { return };
                        if done { line } else { format!("no scan · {line}") }
                    }
                    // wait-open: the script clicked Open Sessions or reopened the app from outside
                    "open" | "wait-open" => {
                        if command == "open" {
                            autoplay_menu_item(&this, cx, ResidentCommand::OpenSessions);
                        }
                        let shown = autoplay_wait(&this, cx, 100, |this, _| !this.window_hidden).await;
                        cx.background_executor().timer(Duration::from_millis(500)).await;
                        let Ok(state) = this.update_in(cx, |this, window, cx| {
                            let table_focused = this.table.read(cx).focus_handle(cx).is_focused(window);
                            format!("shown={shown} active={} table-focused={table_focused} · {}", window.is_window_active(), this.resident_line())
                        }) else {
                            return;
                        };
                        state
                    }
                    "quit" => {
                        println!("autoplay-resident: quit · through the menu item");
                        autoplay_menu_item(&this, cx, ResidentCommand::Quit);
                        return;
                    }
                    other => format!("unknown command {other}"),
                };
                println!("autoplay-resident: {command} · {note}");
                base = this.update(cx, |this, _| this.scans_done).unwrap_or(base);
            }
        })
    }

    /// The menu bar item's title and its menu's three lines: `$0 | Today  $0.09 | In 53k · Out 1.2k | Scanned 15:53`.
    fn resident_line(&self) -> String {
        match &self.resident_view {
            Some(v) => format!("{} | {} | {} | {}", v.title, v.today, v.tokens, v.scanned),
            None => "—".into(),
        }
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

    /// Keyboard focus to what is on screen: the table, or the Usage view.
    pub(crate) fn focus_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.view {
            MainView::Sessions => self.focus_table(window, cx),
            MainView::Usage => self.usage_view.focus.focus(window, cx),
        }
    }

    /// `⌃1` / `⌃2`, the View menu, the toolbar's switch: show the session table or the Usage
    /// view, with the keys going to it. Usage is built again and scanned for (unless a scan ran
    /// less than 30 s ago); back on the session table a search that went stale meanwhile runs.
    pub(crate) fn set_view(&mut self, view: MainView, window: &mut Window, cx: &mut Context<Self>) {
        if self.view != view {
            println!("view: {view:?}");
            self.view = view;
            match view {
                MainView::Usage => {
                    self.refresh_usage(cx);
                    self.auto_scan("usage shown", cx);
                }
                MainView::Sessions => match self.search_stale.take() {
                    Some(StaleSearch::Refresh) if self.searching() => self.refresh_search(cx),
                    Some(_) if self.searching() => self.run_search(cx),
                    _ => {}
                },
            }
            // the next timer scan in a minute (Usage) or ten (the table)
            self._auto_scan = Self::spawn_auto_scan(cx);
            self.persist(cx);
        }
        self.focus_view(window, cx);
        cx.notify();
    }

    /// Remember that the search has to run again once the session table is back.
    fn search_went_stale(&mut self, stale: StaleSearch) {
        self.search_stale = self.search_stale.max(Some(stale));
    }

    /// A click on `Today $…` in the status bar: today in the Usage view.
    pub(crate) fn show_todays_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_usage_range(UsageRange::Today, cx);
        self.set_view(MainView::Usage, window, cx);
    }

    /// How long the idle timer sleeps: a minute while the Usage view is on screen.
    fn auto_scan_interval(&self) -> Duration {
        if self.view == MainView::Usage && !self.window_hidden { USAGE_SCAN_IDLE } else { AUTO_SCAN_IDLE }
    }

    /// The idle timer: scans every `auto_scan_interval` (read again each time). Replacing the
    /// task restarts the wait.
    fn spawn_auto_scan(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                let Ok(wait) = this.update(cx, |this, _| this.auto_scan_interval()) else { break };
                cx.background_executor().timer(wait).await;
                let alive = this.update(cx, |this, cx| {
                    this.roll_day(now_ms(), cx);
                    this.auto_scan("timer", cx);
                });
                if alive.is_err() {
                    break;
                }
            }
        })
    }

    /// Escape in the search box: drop the search and go back to the table.
    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |input, cx| input.set_value("", window, cx));
        self.run_search(cx);
        self.focus_table(window, cx);
    }

    /// Show or hide table columns (the header menu); the next render gives Preview the
    /// difference.
    pub(crate) fn update_columns(&mut self, change: impl FnOnce(&mut SessionTableDelegate), cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            change(table.delegate_mut());
            table.refresh(cx);
            // hiding the sorted column re-sorts the rows: the table observer moves the selection
            // along with its session
            cx.notify();
        });
        self.persist(cx);
        cx.notify();
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

    /// `42 / 5488 sessions · $123.45`: the listed sessions, what they cost (and would cost on
    /// another model).
    fn status_counts(&self, cx: &App) -> String {
        let delegate = self.table.read(cx).delegate();
        let (shown, total, cost) = (delegate.ix.len(), delegate.rows.len(), delegate.shown_cost);
        let projection = delegate.projection().map(|p| format!(" · {} {} ({})", display_name(&p.target), fmt_usd(p.predicted), fmt_change(p.actual, p.predicted))).unwrap_or_default();
        format!("{shown} / {total} sessions · {}{projection}", fmt_usd(cost))
    }

    /// What the status bar's left end reads: `42 / 5488 sessions · $123.45 · Today $203.69` on
    /// the session table; only `Today $203.69` in the Usage view, where the table's counts mean
    /// nothing. `render_status_bar` draws the same, with Today clickable.
    #[cfg(test)]
    fn status_left(&self, cx: &App) -> String {
        match self.view {
            MainView::Usage => self.today_label.to_string(),
            MainView::Sessions if self.today_label.is_empty() => self.status_counts(cx),
            MainView::Sessions => format!("{} · {}", self.status_counts(cx), self.today_label),
        }
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> StatusBar {
        let index: SharedString = match &self.index_status {
            IndexStatus::Loading => "loading index…".into(),
            IndexStatus::Scanning { done, total } => format!("indexing {done} / {total} files").into(),
            IndexStatus::Idle(summary) => summary.clone(),
            IndexStatus::Failed(message) => format!("index error: {message}").into(),
        };
        let bar = StatusBar::new().h(px(22.)).py_0().px(px(10.)).font_family(style::mono(cx)).text_size(px(11.)).text_color(style::head_fg());
        // `Today $…` opens today's usage, from either view
        let today = (!self.today_label.is_empty()).then(|| {
            div()
                .id("status-today")
                .cursor_pointer()
                .text_color(style::text_secondary())
                .hover(|s| s.text_color(style::accent()))
                .child(self.today_label.clone())
                .on_click(cx.listener(|this, _, window, cx| this.show_todays_usage(window, cx)))
        });
        if self.view == MainView::Usage {
            return bar.left(h_flex().children(today)).child(index).right(crate::usage_view::BASIS);
        }
        let (found, took) = self.search_status().unwrap_or_default();
        let status = if found.is_empty() { self.status.clone() } else { found };
        let center: SharedString = if status.is_empty() { index } else { format!("{status} · {index}").into() };
        let took = if took.is_empty() { String::new() } else { format!("{took} · ") };
        let first_rows = self.first_rows_ms.map(|ms| format!(" · first rows in {ms}ms")).unwrap_or_default();
        let counts = format!("{}{}", self.status_counts(cx), if today.is_some() { " · " } else { "" });
        bar.left(h_flex().child(counts).children(today)).child(center).right(format!("{took}{}{first_rows}", self.db_info))
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
            .on_action(cx.listener(|this, _: &ShowSessions, window, cx| this.set_view(MainView::Sessions, window, cx)))
            .on_action(cx.listener(|this, _: &ShowUsage, window, cx| this.set_view(MainView::Usage, window, cx)))
            .on_action(cx.listener(|this, _: &LeaveDay, _, cx| this.leave_day(cx)))
            .on_action(cx.listener(|this, _: &ToggleDetail, _, cx| {
                if this.view == MainView::Usage {
                    return;
                }
                this.detail_visible = !this.detail_visible;
                this.sync_projection(cx);
                this.persist(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleFavorite, window, cx| {
                if this.view == MainView::Sessions {
                    this.toggle_favorite(window, cx);
                }
            }))
            .on_action(cx.listener(|this, action: &favorites::FavoriteSession, window, cx| this.favorite_session(&action.id, window, cx)))
            .on_action(cx.listener(|this, _: &RevealFavoritesFolder, _, cx| cx.reveal_path(&this.favorites.root)))
            .on_action(cx.listener(|this, _: &ChangeFavoritesFolder, window, cx| this.change_favorites_folder(window, cx)))
            .on_action(cx.listener(|this, _: &DefaultFavoritesFolder, window, cx| this.default_favorites_folder(window, cx)))
            .on_action(cx.listener(|this, _: &CloseWindow, window, cx| this.close_requested(window, cx)))
            .on_action(cx.listener(|this, _: &Rescan, _, cx| this.rescan(false, cx)))
            .on_action(cx.listener(|this, _: &RebuildIndex, _, cx| this.rescan(true, cx)))
            .on_action(cx.listener(|this, _: &ToggleEmptySessions, _, cx| {
                if this.view == MainView::Sessions {
                    this.toggle_empty_sessions(cx);
                }
            }))
            .on_action(cx.listener(|this, action: &HideProject, _, cx| {
                if this.view == MainView::Sessions {
                    this.hide_project_of(action.id.as_deref(), cx);
                }
            }))
            .on_action(cx.listener(|this, action: &ToggleColumn, _, cx| this.update_columns(|d| d.toggle_column(&action.key), cx)))
            .on_action(cx.listener(|this, _: &ShowAllColumns, _, cx| this.update_columns(|d| d.set_hidden_columns([]), cx)))
            .on_action(cx.listener(|this, action: &CopySessionId, window, cx| this.copy_session_id(action.id.as_deref(), window, cx)))
            .on_action(cx.listener(|this, action: &CopyResume, window, cx| this.copy_resume(action.id.as_deref(), window, cx)))
            .on_action(cx.listener(|this, action: &RevealTranscript, window, cx| this.reveal_transcript(action.id.as_deref(), window, cx)))
            .on_action(cx.listener(|this, action: &OpenProjectFolder, window, cx| this.open_project_folder(action.id.as_deref(), window, cx)))
            .on_action(cx.listener(|this, action: &ExportTranscript, window, cx| this.export_transcript(action.id.as_deref(), window, cx)))
            // ⌘F in Usage: back to the table, into the search box
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.set_view(MainView::Sessions, window, cx);
                this.focus_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Escape, window, cx| match this.view {
                MainView::Sessions => this.clear_search(window, cx),
                MainView::Usage => this.leave_day(cx),
            }))
            .on_action(cx.listener(|this, _: &ToggleRegex, _, cx| {
                if this.view == MainView::Sessions {
                    this.toggle_regex(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleCase, _, cx| {
                if this.view == MainView::Sessions {
                    this.toggle_case(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextHit, _, cx| {
                if this.view == MainView::Sessions {
                    this.step_hit(true, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PrevHit, _, cx| {
                if this.view == MainView::Sessions {
                    this.step_hit(false, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextTurn, _, cx| {
                if this.view == MainView::Sessions {
                    this.step_turn(true, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PrevTurn, _, cx| {
                if this.view == MainView::Sessions {
                    this.step_turn(false, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Preview1, _, cx| {
                if this.view == MainView::Sessions {
                    this.set_mode(DisplayMode::Title, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Preview2, _, cx| {
                if this.view == MainView::Sessions {
                    this.set_mode(DisplayMode::FirstPrompt, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Preview3, _, cx| {
                if this.view == MainView::Sessions {
                    this.set_mode(DisplayMode::LastPrompt, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Preview4, _, cx| {
                if this.view == MainView::Sessions {
                    this.set_mode(DisplayMode::LastMessage, cx);
                }
            }))
            .child(self.render_toolbar(cx))
            .when(self.view == MainView::Usage, |el| el.child(self.render_usage(window, cx)))
            .when(self.view == MainView::Sessions, |el| el.child(
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
            ))
            .child(self.render_status_bar(cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

// Only the icons the component bundle lacks (the detail header's buttons); see gpui-kit-assets'
// README, "Default component icons and application extras".
gpui_kit::assets::icon_assets!(ExtraIcons, [SquareTerminal, FileSearch, Download, ArrowUpRight]);

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

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Autoplay: look every 100ms until `ready`, at most `tries` times. False when it never was (or
/// the app is gone).
async fn autoplay_wait(this: &WeakEntity<AppRoot>, cx: &mut AsyncWindowContext, tries: usize, ready: impl Fn(&AppRoot, &App) -> bool) -> bool {
    for _ in 0..tries {
        match this.update(cx, |this, cx| ready(this, cx)) {
            Ok(true) => return true,
            Ok(false) => cx.background_executor().timer(Duration::from_millis(100)).await,
            Err(_) => return false,
        }
    }
    false
}

/// Autoplay: what a click on the menu bar item's menu item does (its target sends the command
/// over the channel); straight to `on_resident_command` without a menu bar item.
fn autoplay_menu_item(this: &WeakEntity<AppRoot>, cx: &mut AsyncWindowContext, command: ResidentCommand) {
    this.update_in(cx, |this, window, cx| match cx.try_global::<Resident>() {
        Some(resident) => resident.perform(command),
        None => this.on_resident_command(command, window, cx),
    })
    .ok();
}

/// The `n`-th command dropped into the autoplay's gate directory: the file `<n>-<command>`.
fn gate_command(gate: &std::path::Path, n: usize) -> Option<String> {
    let prefix = format!("{n}-");
    std::fs::read_dir(gate).ok()?.filter_map(|entry| entry.ok()).find_map(|entry| entry.file_name().to_str()?.strip_prefix(&prefix).map(str::to_string))
}

/// `ASM_AUTOPLAY_STEP`, or `cut-output` for the older `ASM_AUTOPLAY_CUT_OUTPUT=1`.
fn autoplay_step() -> Option<String> {
    std::env::var("ASM_AUTOPLAY_STEP").ok().filter(|v| !v.is_empty()).or_else(|| std::env::var_os("ASM_AUTOPLAY_CUT_OUTPUT").map(|_| "cut-output".into()))
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-d", ToggleFavorite, None),
        KeyBinding::new("cmd-q", Quit, None),
        // not AppKit's performClose: (that asks windowShouldClose: inside a GPUI update, and the
        // app aborts): straight to `close_requested`
        KeyBinding::new("cmd-w", CloseWindow, None),
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
        // not ⌘1–⌘4: those are the Preview modes. In the search box too.
        KeyBinding::new("ctrl-1", ShowSessions, None),
        KeyBinding::new("ctrl-2", ShowUsage, None),
        KeyBinding::new("escape", LeaveDay, Some(crate::usage_view::KEY_CONTEXT)),
    ]);
}

/// About, Check for Updates… (only when the bundle carries Sparkle), Quit.
fn app_menu_items(updates: bool) -> Vec<MenuItem> {
    let mut items = vec![MenuItem::action("About Agent Session Manager", About)];
    if updates {
        items.push(MenuItem::action("Check for Updates…", CheckForUpdates));
    }
    items.extend([MenuItem::separator(), MenuItem::action("Quit", Quit)]);
    items
}

fn main() {
    // closing the window hides it (`close_requested`); only Quit ends the app
    let app = gpui_kit::application().with_assets(AppAssets).with_quit_mode(QuitMode::Explicit);
    let (menu, menu_commands) = async_channel::unbounded();
    // opened again from Finder or Spotlight: AppKit asks only while no window is visible. The
    // user asked for the app just now: to the front here, not later in the GPUI task
    let reopen = menu.clone();
    app.on_reopen(move |_| {
        resident::bring_forward();
        let _ = reopen.try_send(ResidentCommand::Reopen);
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        style::install_theme(cx);
        cx.activate(true);
        cx.on_action(|_: &Quit, cx| cx.quit());
        bind_keys(cx);
        let updates = updater::install(cx);
        resident::install(cx, menu);
        cx.set_menus([
            Menu::new("Agent Session Manager").items(app_menu_items(updates)),
            Menu::new("File").items(vec![MenuItem::action("Close Window", CloseWindow)]),
            Menu::new("View").items(vec![
                MenuItem::action("Sessions", ShowSessions),
                MenuItem::action("Usage", ShowUsage),
                MenuItem::separator(),
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
                let view = cx.new(|cx| {
                    let mut app = AppRoot::new(AppPaths::from_env(), window, cx);
                    app.listen_to_menu(menu_commands, window, cx);
                    app
                });
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
