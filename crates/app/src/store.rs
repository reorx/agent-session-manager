//! Bridge to `crates/core`: one background thread owns the SQLite index (a `Db` never leaves its
//! thread), serves requests from the UI and streams `StoreEvent`s back over an async channel.
//!
//! Start-up sequence: open the index → `Loaded` with whatever is already indexed (hot start) →
//! incremental scan, streaming `Progress` and every written `Session` (cold start fills the
//! table this way) → `ScanDone` → a second `Loaded` when the scan changed anything, so rows that
//! were streamed before their project's path was known end up exact.
//! The first load includes the local model catalog; scans also reload when its windows change.
//! Right after the first `Loaded` comes `Usage { full: true }`: every session in the usage ledger
//! (or `UsageUnavailable`). Every scan then sends `Usage { full: false }` with the ledger's rows
//! of the sessions it wrote, before its `ScanDone`. A quiet scan (`Scan { quiet: true }`, the
//! app's automatic ones) sends no `Progress`, and its `Session`s only once it committed; one that
//! finds the index or the usage ledger locked by another process ends in `ScanSkipped` alone
//! instead of `Failed`.
//!
//! A second thread, `asm-reader`, reads transcripts for the detail panel straight from the file
//! (no `Db`), so a conversation shows up while a scan holds the store thread. It also reads the
//! prompts of a history-only session from `history.jsonl`, and writes TXT exports. Conversation
//! and history requests supersede each other like searches do (`seq`, see `search.rs`); exports
//! all run. Its `Conversation` and `Exported` events go out on the same channel.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use agent_session_core::claude::conversation::{conversation_from_history, read_conversation};
use agent_session_core::claude::history::history_prompts;
use agent_session_core::claude::paths::ClaudeDir;
use agent_session_core::claude::model_catalog::ModelCatalog;
use agent_session_core::index::{Db, IndexStats, Indexer, ListFilter, Order, Progress, ScanEvent, ScanReport, is_busy};
use agent_session_core::model::{Conversation, SessionSummary, SessionUsage};
use agent_session_core::search::Matcher;
use agent_session_core::transcript::read_transcript;
use anyhow::{Result, anyhow, ensure};
use agent_session_core::{config, favorites::{self, FavoritesDir}};
use async_channel::{Receiver, Sender};
use chrono::Local;

use crate::conversation::{self, ConversationView};

pub enum StoreRequest {
    Favorite(String),
    Unfavorite(String),
    SetFavoritesDir(Option<PathBuf>),
    /// An incremental scan. `quiet` ones (automatic) show nothing until they are through (no
    /// `Progress`, `Session`s after the commit) and give up without a `Failed` when another
    /// process holds the index or the ledger (`ScanSkipped`).
    Scan { quiet: bool },
    Rebuild,
}

#[derive(Debug)]
pub enum StoreEvent {
    /// Everything in the index, replacing whatever the UI holds.
    Loaded { sessions: Vec<SessionSummary>, catalog: Arc<ModelCatalog>, elapsed: Duration },
    Progress(Progress),
    /// One session written by the running scan (new or changed).
    Session(SessionSummary),
    /// `quiet`: the scan was asked for with `Scan { quiet: true }`.
    ScanDone { report: ScanReport, stats: IndexStats, quiet: bool },
    /// A quiet scan found the index or the usage ledger locked by another process and gave up
    /// (rolled back; it sent nothing before this); the next one tries again.
    ScanSkipped,
    /// Usage ledger rows: every session in the ledger (`full`, once after the first `Loaded`),
    /// or those of the sessions a scan wrote (always before its `ScanDone`). Sessions a scan did
    /// not write keep what the UI has: the ledger never forgets a call.
    Usage { sessions: Vec<SessionUsage>, full: bool },
    /// This run has no usage ledger (one written by a newer version, or it cannot be read).
    UsageUnavailable,
    /// A transcript read for the detail panel (`Store::read_conversation`), or the prompts of a
    /// history-only session (`Store::read_history`), already built into cards. `read` is the
    /// file read, `build` the conversion; both ran on the reader thread.
    Conversation { seq: u64, session_id: String, conversation: Result<Arc<ConversationView>, String>, read: Duration, build: Duration },
    /// A TXT export (`Store::export`) was written to `dest`: the lines of the transcript that
    /// could not be parsed and were left out, or what went wrong.
    Exported { dest: PathBuf, result: Result<usize, String> },
    FavoriteChanged { id: String, favorite: bool },
    FavoritesDirChanged { dir: PathBuf, moved: usize, skipped: usize },
    RebuildingRoots,
    Failed(String),
}

pub struct Store {
    requests: mpsc::Sender<StoreRequest>,
    reads: mpsc::Sender<ReadRequest>,
    latest: Arc<AtomicU64>,
}

enum ReadRequest {
    Conversation { seq: u64, session_id: String, path: PathBuf, project_path: String, matcher: Option<Matcher> },
    History { seq: u64, session_id: String, project_key: String, matcher: Option<Matcher> },
    Export { src: PathBuf, dest: PathBuf },
}

impl ReadRequest {
    /// Requests for the detail panel have one; a newer one supersedes them. Exports have none.
    fn seq(&self) -> Option<u64> {
        match self {
            ReadRequest::Conversation { seq, .. } | ReadRequest::History { seq, .. } => Some(*seq),
            ReadRequest::Export { .. } => None,
        }
    }
}

impl Store {
    pub fn spawn(claude: ClaudeDir, db_path: PathBuf, favorites: FavoritesDir) -> (Store, Receiver<StoreEvent>) {
        let (requests, request_rx) = mpsc::channel();
        let (event_tx, events) = async_channel::unbounded();
        let (reads, read_rx) = mpsc::channel();
        let latest = Arc::new(AtomicU64::new(0));
        let (reader_latest, reader_events, reader_claude) = (latest.clone(), event_tx.clone(), claude.clone());
        std::thread::Builder::new()
            .name("asm-store".into())
            .spawn(move || run(claude, db_path, favorites, request_rx, event_tx))
            .expect("failed to spawn the store thread");
        std::thread::Builder::new()
            .name("asm-reader".into())
            .spawn(move || serve_reads(read_rx, reader_latest, reader_events, reader_claude))
            .expect("failed to spawn the reader thread");
        (Store { requests, reads, latest }, events)
    }

    /// False when the store thread is gone (it stops when the index cannot be opened): the
    /// request is dropped and no event will ever answer it.
    pub fn request(&self, request: StoreRequest) -> bool {
        self.requests.send(request).is_ok()
    }

    /// Read a transcript for the detail panel on the reader thread, superseding any read that
    /// has not started yet. Returns the `seq` the `Conversation` event carries; events of older
    /// requests are stale from now on.
    pub fn read_conversation(&self, session_id: String, path: PathBuf, project_path: String, matcher: Option<Matcher>) -> u64 {
        let seq = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        self.reads.send(ReadRequest::Conversation { seq, session_id, path, project_path, matcher }).ok();
        seq
    }

    /// Like `read_conversation`, for a history-only session: its prompts in `history.jsonl`,
    /// one card each.
    pub fn read_history(&self, session_id: String, project_key: String, matcher: Option<Matcher>) -> u64 {
        let seq = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        self.reads.send(ReadRequest::History { seq, session_id, project_key, matcher }).ok();
        seq
    }

    /// Write the TXT export of the transcript at `src` to `dest` on the reader thread; answered
    /// by `Exported`. Never superseded.
    pub fn export(&self, src: PathBuf, dest: PathBuf) {
        self.reads.send(ReadRequest::Export { src, dest }).ok();
    }
}

/// Reader thread body: of the conversation and history requests only the newest is read;
/// whatever queued up before it, or was superseded while being read, is dropped. Every export
/// runs, in order. Never touches the index, so a scan never delays it.
fn serve_reads(requests: mpsc::Receiver<ReadRequest>, latest: Arc<AtomicU64>, events: Sender<StoreEvent>, claude: ClaudeDir) {
    let stale = |seq: u64| latest.load(Ordering::SeqCst) != seq;
    while let Ok(first) = requests.recv() {
        let mut queued = vec![first];
        queued.extend(requests.try_iter());
        let newest = queued.iter().rposition(|r| r.seq().is_some());
        for (ix, request) in queued.into_iter().enumerate() {
            let event = match request {
                ReadRequest::Export { src, dest } => export(src, dest),
                _ if Some(ix) != newest || request.seq().is_some_and(stale) => continue,
                request => read(request, &claude),
            };
            if !event_is_stale(&event, &stale) {
                events.send_blocking(event).ok();
            }
        }
    }
}

fn event_is_stale(event: &StoreEvent, stale: impl Fn(u64) -> bool) -> bool {
    matches!(event, StoreEvent::Conversation { seq, .. } if stale(*seq))
}

fn read(request: ReadRequest, claude: &ClaudeDir) -> StoreEvent {
    let started = Instant::now();
    let (seq, session_id, project_path, matcher, conversation) = match request {
        ReadRequest::Conversation { seq, session_id, path, project_path, matcher } => {
            let conversation = read_conversation(&path, matcher.as_ref());
            (seq, session_id, project_path, matcher, conversation)
        }
        ReadRequest::History { seq, session_id, project_key, matcher } => {
            let conversation = history_prompts(claude, &project_key, &session_id).map(|prompts| conversation_from_history(&prompts, matcher.as_ref()));
            (seq, session_id, String::new(), matcher, conversation)
        }
        ReadRequest::Export { .. } => unreachable!("exports are not read for the detail panel"),
    };
    let read = started.elapsed();
    let conversation = conversation.map(|conv| Arc::new(view_of(&conv, &project_path, matcher.as_ref()))).map_err(|e| format!("{e:#}"));
    StoreEvent::Conversation { seq, session_id, conversation, read, build: started.elapsed() - read }
}

fn view_of(conv: &Conversation, project_path: &str, matcher: Option<&Matcher>) -> ConversationView {
    let mut view = conversation::build(conv, project_path, Local::now().date_naive());
    view.pattern = matcher.map(|m| m.pattern().to_string());
    view
}

fn export(src: PathBuf, dest: PathBuf) -> StoreEvent {
    let result = read_transcript(&src).and_then(|export| {
        std::fs::write(&dest, export.text).map_err(|e| anyhow!("write {}: {e}", dest.display()))?;
        Ok(export.skipped_lines)
    });
    StoreEvent::Exported { dest, result: result.map_err(|e| format!("{e:#}")) }
}

/// Thread body: every failure becomes a `Failed` event instead of taking the thread down.
fn run(claude: ClaudeDir, db_path: PathBuf, favorites: FavoritesDir, requests: mpsc::Receiver<StoreRequest>, events: Sender<StoreEvent>) {
    let mut worker = match Worker::open(claude, &db_path, favorites, events.clone()) {
        Ok(worker) => worker,
        Err(e) => {
            // refuse requests before saying so: one sent after the app heard of the failure must
            // not wait in the channel for an answer that never comes (`Store::request`)
            drop(requests);
            events.send_blocking(StoreEvent::Failed(format!("open index: {e:#}"))).ok();
            return;
        }
    };
    let fail = |e: anyhow::Error| events.send_blocking(StoreEvent::Failed(format!("{e:#}"))).ok();
    let started = worker.load().and_then(|_| {
        if let Err(e) = worker.load_usage() {
            // the index works without the ledger: say so and go on
            events.send_blocking(StoreEvent::UsageUnavailable).ok();
            fail(e);
        }
        worker.scan(false, false)
    });
    if let Err(e) = started {
        fail(e);
    }
    for request in requests {
        worker.serve(request);
    }
}

struct Worker {
    claude: ClaudeDir,
    catalog: Arc<ModelCatalog>,
    favorites: FavoritesDir,
    db: Db,
    indexer: Indexer,
    events: Sender<StoreEvent>,
}

impl Worker {
    fn open(claude: ClaudeDir, db_path: &std::path::Path, favorites: FavoritesDir, events: Sender<StoreEvent>) -> Result<Worker> {
        let started = Instant::now();
        let catalog = Arc::new(ModelCatalog::load(&claude));
        log_catalog(&catalog, started.elapsed());
        Ok(Worker { db: Db::open(db_path)?, indexer: Indexer::new(claude.clone(), Some(favorites.claude())), claude, catalog, favorites, events })
    }

    /// One request; a failure becomes `Failed`, or `ScanSkipped` for a quiet scan that found the
    /// index or the ledger busy.
    fn serve(&mut self, request: StoreRequest) {
        let quiet = matches!(request, StoreRequest::Scan { quiet: true });
        let result = match request {
            StoreRequest::Favorite(id) => self.favorite(id, false),
            StoreRequest::Unfavorite(id) => self.favorite(id, true),
            StoreRequest::SetFavoritesDir(dir) => self.set_favorites_dir(dir),
            StoreRequest::Scan { quiet } => self.scan(false, quiet),
            StoreRequest::Rebuild => self.scan(true, false),
        };
        let event = match result {
            Ok(()) => return,
            Err(e) if quiet && is_busy(&e) => {
                println!("scan skipped: {e:#}");
                StoreEvent::ScanSkipped
            }
            Err(e) => StoreEvent::Failed(format!("{e:#}")),
        };
        self.events.send_blocking(event).ok();
    }

    fn favorite(&mut self, id: String, remove: bool) -> Result<()> {
        let s = self.db.session(&id)?.ok_or_else(|| anyhow!("session not found"))?;
        if remove { favorites::remove(&self.favorites.root, &s.project_key, &id, &self.favorites.trash)?; }
        else {
            ensure!(!s.history_only(), "cannot favorite a history-only session");
            favorites::save(&self.favorites.root, s.file_path.as_ref().ok_or_else(|| anyhow!("no transcript"))?, &s.project_key, &id)?;
        }
        self.scan(false, false)?;
        self.events.send_blocking(StoreEvent::FavoriteChanged { id, favorite: !remove })?; Ok(())
    }
    fn set_favorites_dir(&mut self, dir: Option<PathBuf>) -> Result<()> {
        ensure!(!config::environment_override(), "由环境变量 ASM_FAVORITES_DIR 指定");
        let db_path = self.db.path().unwrap().to_path_buf();
        let new = config::canonical_root(&dir.clone().unwrap_or_else(|| config::default_favorites_dir(&db_path)))?;
        let report = favorites::migrate_with_commit(&self.favorites.root, &new, &self.claude, || self.db.transaction(|db| {
            db.set_meta("favorites_root", &config::canonical_root(&new.join("claude"))?.to_string_lossy())?;
            config::set_favorites_dir(&db_path, dir.as_ref().map(|_| new.as_path()))
        }))?;
        self.favorites.root = new.clone(); self.indexer = Indexer::new(self.claude.clone(), Some(self.favorites.claude()));
        self.scan(false, false)?;
        self.events.send_blocking(StoreEvent::FavoritesDirChanged { dir: new, moved: report.moved, skipped: report.skipped })?; Ok(())
    }

    fn load(&self) -> Result<()> {
        let t = Instant::now();
        let sessions = self.db.list_sessions(&ListFilter { order: Order::UpdatedDesc, ..Default::default() })?;
        self.events.send_blocking(StoreEvent::Loaded { sessions, catalog: self.catalog.clone(), elapsed: t.elapsed() })?;
        Ok(())
    }

    /// Every session in the usage ledger, or `UsageUnavailable` when this run has none.
    fn load_usage(&self) -> Result<()> {
        let event = match self.db.ledger_usage(None)? {
            Some(sessions) => StoreEvent::Usage { sessions, full: true },
            None => StoreEvent::UsageUnavailable,
        };
        self.events.send_blocking(event)?;
        Ok(())
    }

    /// A rebuild empties the index but not the ledger, so the UI keeps the usage it has.
    fn scan(&mut self, rebuild: bool, quiet: bool) -> Result<()> {
        if rebuild {
            self.db.clear_all()?;
            self.load()?;
        }
        let events = self.events.clone();
        let mut written = Vec::new();
        // A quiet scan shows nothing until it is through: no progress, and the rows it wrote only
        // once its transaction committed (one that waited for a lock and gave up rolled back).
        let mut held = Vec::new();
        let result = self.indexer.scan_streaming(&mut self.db, &mut |ev| match ev {
            ScanEvent::Progress(_) if quiet => {}
            ScanEvent::Progress(p) => {
                events.send_blocking(StoreEvent::Progress(p)).ok();
            }
            ScanEvent::Session(s) => {
                written.push(s.id.clone());
                if quiet {
                    held.push(s);
                } else {
                    events.send_blocking(StoreEvent::Session(s)).ok();
                }
            }
        });
        let report = match result {
            Err(e) if e.is::<agent_session_core::index::indexer::RootsChanged>() => {
                self.events.send_blocking(StoreEvent::RebuildingRoots)?;
                return self.scan(true, false);
            }
            result => result?,
        };
        for s in held {
            self.events.send_blocking(StoreEvent::Session(s))?;
        }
        if let Some(sessions) = self.db.ledger_usage(Some(&written))?.filter(|s| !s.is_empty()) {
            self.events.send_blocking(StoreEvent::Usage { sessions, full: false })?;
        }
        let changed = report.files_seen != report.files_unchanged || report.files_missing > 0 || report.history_sessions > 0;
        self.events.send_blocking(StoreEvent::ScanDone { report, stats: self.db.stats()?, quiet })?;
        let started = Instant::now();
        let catalog = Arc::new(ModelCatalog::load(&self.claude));
        let catalog_changed = catalog.windows() != self.catalog.windows();
        if catalog_changed {
            log_catalog(&catalog, started.elapsed());
        }
        self.catalog = catalog;
        if changed || catalog_changed {
            self.load()?;
        }
        Ok(())
    }
}

fn log_catalog(catalog: &ModelCatalog, elapsed: Duration) {
    let skipped = catalog.files().iter().filter(|f| f.error.is_some()).count();
    println!("model catalog: {} models from {} files ({} skipped) in {:.3} ms",
        catalog.windows().len(), catalog.files().len(), skipped, elapsed.as_secs_f64() * 1000.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::Card;

    const S1: &str = "11111111-aaaa-4bbb-8ccc-000000000001";

    fn write_catalog(claude: &ClaudeDir, window: u64, fetched: i64) {
        std::fs::create_dir_all(claude.model_catalog_dir()).unwrap();
        let value = serde_json::json!({"fetchedAt": fetched, "catalog": {"config": {"models": [
            {"id": "claude-opus-9", "runtime": {"max_input_tokens": window}}
        ]}}});
        std::fs::write(claude.model_catalog_dir().join("test-cc.json"), value.to_string()).unwrap();
    }

    #[test]
    fn the_first_load_includes_the_catalog_even_with_no_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let claude = ClaudeDir::new(dir.path().join("claude"));
        write_catalog(&claude, 1_000_000, 1);
        let (_store, events) = spawn(claude, dir.path().join("index.sqlite"));
        assert!(matches!(events.recv_blocking().unwrap(), StoreEvent::Loaded { sessions, catalog, .. }
            if sessions.is_empty() && catalog.window("claude-opus-9") == Some(1_000_000)));
    }

    #[test]
    fn scans_reload_only_when_catalog_windows_change_including_removal() {
        let dir = tempfile::tempdir().unwrap();
        let claude = ClaudeDir::new(dir.path().join("claude"));
        write_catalog(&claude, 1_000_000, 1);
        let (tx, events) = async_channel::unbounded();
        let favorites = FavoritesDir { root: dir.path().join("favorites"), trash: favorites::Trash::Dir(dir.path().join("trash")) };
        let mut worker = Worker::open(claude.clone(), &dir.path().join("index.sqlite"), favorites, tx).unwrap();
        worker.scan(false, false).unwrap();
        let drain = || std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
        assert!(!drain().iter().any(|e| matches!(e, StoreEvent::Loaded { .. })));
        for (window, fetched, reload) in [(500_000, 2, true), (500_000, 3, false)] {
            write_catalog(&claude, window, fetched);
            worker.scan(false, false).unwrap();
            let seen = drain();
            assert_eq!(seen.iter().filter(|e| matches!(e, StoreEvent::Loaded { .. })).count(), usize::from(reload));
            let scan = seen.iter().position(|e| matches!(e, StoreEvent::ScanDone { .. })).unwrap();
            if reload {
                assert!(matches!(seen.get(scan + 1), Some(StoreEvent::Loaded { sessions, catalog, .. })
                    if sessions.is_empty() && catalog.window("claude-opus-9") == Some(window)));
            }
        }
        std::fs::remove_dir_all(claude.model_catalog_dir()).unwrap();
        worker.scan(false, false).unwrap();
        assert!(drain().iter().any(|e| matches!(e, StoreEvent::Loaded { catalog, .. } if catalog.windows().is_empty())));
    }

    fn spawn(claude: ClaudeDir, db: PathBuf) -> (Store, Receiver<StoreEvent>) {
        let root = db.with_file_name("favorites"); let trash = favorites::Trash::Dir(db.with_file_name("trash"));
        Store::spawn(claude, db, FavoritesDir { root, trash })
    }

    #[test]
    fn favorite_and_unfavorite_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let (store, events) = spawn(fixture_claude(), dir.path().join("index.sqlite")); until_settled(&events);
        for (request, expected) in [(StoreRequest::Favorite(S1.into()), true), (StoreRequest::Unfavorite(S1.into()), false)] {
            store.request(request);
            loop {
                match events.recv_blocking().unwrap() {
                    StoreEvent::FavoriteChanged { favorite, .. } => { assert_eq!(favorite, expected); break; }
                    StoreEvent::Failed(e) => panic!("{e}"),
                    _ => {}
                }
            }
        }
    }

    fn fixture_claude() -> ClaudeDir {
        ClaudeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../core/tests/fixtures/claude"))
    }

    /// Events up to and including the `Loaded` that follows a scan (or `ScanDone` if nothing changed).
    fn until_settled(events: &Receiver<StoreEvent>) -> Vec<StoreEvent> {
        let mut seen = Vec::new();
        loop {
            let ev = events.recv_blocking().expect("store thread ended early");
            let scan_changed = match &ev {
                StoreEvent::ScanDone { report, .. } => Some(report.files_seen != report.files_unchanged || report.history_sessions > 0),
                _ => None,
            };
            let reloaded = matches!(ev, StoreEvent::Loaded { .. }) && seen.iter().any(|e| matches!(e, StoreEvent::ScanDone { .. }));
            seen.push(ev);
            if scan_changed == Some(false) || reloaded {
                return seen;
            }
        }
    }

    fn loaded_counts(events: &[StoreEvent]) -> Vec<usize> {
        events.iter().filter_map(|e| if let StoreEvent::Loaded { sessions, .. } = e { Some(sessions.len()) } else { None }).collect()
    }

    fn streamed(events: &[StoreEvent]) -> usize {
        events.iter().filter(|e| matches!(e, StoreEvent::Session(_))).count()
    }

    #[test]
    fn a_cold_start_shows_an_empty_table_then_streams_every_session_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, events) = spawn(fixture_claude(), dir.path().join("index.sqlite"));
        let seen = until_settled(&events);
        assert!(matches!(seen.first(), Some(StoreEvent::Loaded { sessions, .. }) if sessions.is_empty()));
        assert_eq!(streamed(&seen), 8, "4 transcripts + 4 history-only sessions");
        assert!(seen.iter().any(|e| matches!(e, StoreEvent::Progress(p) if p.done == p.total && p.total == 4)));
        assert_eq!(loaded_counts(&seen), [0, 8], "the final reload carries the exact rows");
        assert!(!seen.iter().any(|e| matches!(e, StoreEvent::Failed(_))));
    }

    #[test]
    fn a_hot_start_loads_from_the_index_and_the_scan_stays_quiet() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("index.sqlite");
        {
            let (_store, events) = spawn(fixture_claude(), db_path.clone());
            until_settled(&events);
        }
        let (_store, events) = spawn(fixture_claude(), db_path);
        let seen = until_settled(&events);
        assert_eq!(loaded_counts(&seen), [8], "one load, no reload");
        assert_eq!(streamed(&seen), 0);
        let Some(StoreEvent::Loaded { sessions, .. }) = seen.first() else { panic!("first event must be Loaded") };
        assert!(sessions.windows(2).all(|w| w[0].updated_at >= w[1].updated_at), "newest first");
    }

    #[test]
    fn a_rebuild_streams_everything_again() {
        let dir = tempfile::tempdir().unwrap();
        let (store, events) = spawn(fixture_claude(), dir.path().join("index.sqlite"));
        until_settled(&events);

        store.request(StoreRequest::Rebuild);
        let seen = until_settled(&events);
        assert_eq!(loaded_counts(&seen), [0, 8], "the table empties, then fills again");
        assert_eq!(streamed(&seen), 8);
    }

    const S2: &str = "22222222-aaaa-4bbb-8ccc-000000000002";
    const ALPHA: &str = "/Users/me/Code/alpha";

    fn transcript(id: &str) -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../core/tests/fixtures/claude/projects/-Users-me-Code-alpha")).join(format!("{id}.jsonl"))
    }

    /// Events up to and including the `Conversation` of `seq`.
    fn until_conversation(events: &Receiver<StoreEvent>, seq: u64) -> Vec<StoreEvent> {
        let mut seen = Vec::new();
        loop {
            let ev = events.recv_blocking().expect("store threads ended early");
            let done = matches!(&ev, StoreEvent::Conversation { seq: s, .. } if *s == seq);
            seen.push(ev);
            if done {
                return seen;
            }
        }
    }

    fn conversation_of(ev: &StoreEvent) -> (u64, &str, &Result<Arc<ConversationView>, String>) {
        let StoreEvent::Conversation { seq, session_id, conversation, .. } = ev else { panic!("expected a Conversation, got {ev:?}") };
        (*seq, session_id, conversation)
    }

    #[test]
    fn a_conversation_is_served_on_request() {
        let dir = tempfile::tempdir().unwrap();
        let (store, events) = spawn(fixture_claude(), dir.path().join("index.sqlite"));
        until_settled(&events);

        let seq = store.read_conversation(S1.into(), transcript(S1), ALPHA.into(), None);
        let seen = until_conversation(&events, seq);
        let (_, session_id, view) = conversation_of(seen.last().unwrap());
        let view = view.as_ref().unwrap();
        assert_eq!(session_id, S1);
        assert_eq!((view.summary.as_ref(), view.cards.len()), ("4 turns · 3 tool calls · 1 cache rebuild", 4), "read from the transcript, built into cards; the fixture's last call came back a day later");
        assert!(view.pattern.is_none() && view.hits.is_empty());

        let matcher = Matcher::words("parser").unwrap();
        let seq = store.read_conversation(S1.into(), transcript(S1), ALPHA.into(), Some(matcher.clone()));
        let seen = until_conversation(&events, seq);
        let view = conversation_of(seen.last().unwrap()).2.as_ref().unwrap();
        assert_eq!(view.pattern.as_deref(), Some(matcher.pattern()), "the view says which search it was marked for");
        assert!(!view.hits.is_empty());
    }

    #[test]
    fn a_conversation_is_served_before_the_first_scan_settles() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("index.sqlite");
        // another writer holds the index: the store thread's scan waits for it
        let writer = Db::open(&db_path).unwrap();
        writer.conn().execute_batch("BEGIN IMMEDIATE").unwrap();
        let (store, events) = spawn(fixture_claude(), db_path);

        let asked = Instant::now();
        let seq = store.read_conversation(S1.into(), transcript(S1), ALPHA.into(), None);
        let seen = until_conversation(&events, seq);
        assert!(asked.elapsed() < Duration::from_secs(3), "served at once, not after the busy timeout gave up: {:?}", asked.elapsed());
        assert!(conversation_of(seen.last().unwrap()).2.is_ok());
        assert!(!seen.iter().any(|e| matches!(e, StoreEvent::ScanDone { .. } | StoreEvent::Failed(_))), "the scan is still waiting: {seen:?}");

        writer.conn().execute_batch("COMMIT").unwrap();
        let settled = until_settled(&events);
        assert!(!settled.iter().any(|e| matches!(e, StoreEvent::Failed(_))));
        assert_eq!(streamed(&settled), 8, "and the scan goes on once the index is free");
    }

    #[test]
    fn a_newer_request_supersedes_the_one_before() {
        // queued requests: only the newest one is read
        let (requests, queue) = mpsc::channel();
        let (event_tx, events) = async_channel::unbounded();
        let latest = Arc::new(AtomicU64::new(3));
        for (seq, id) in [(1, S1), (2, S2), (3, S1)] {
            requests.send(ReadRequest::Conversation { seq, session_id: id.into(), path: transcript(id), project_path: ALPHA.into(), matcher: None }).unwrap();
        }
        drop(requests);
        serve_reads(queue, latest, event_tx, fixture_claude());
        let served: Vec<u64> = std::iter::from_fn(|| events.try_recv().ok()).map(|e| conversation_of(&e).0).collect();
        assert_eq!(served, [3]);

        // through the store: whatever arrives for the older request arrives before the newer one
        let dir = tempfile::tempdir().unwrap();
        let (store, events) = spawn(fixture_claude(), dir.path().join("index.sqlite"));
        let old = store.read_conversation(S1.into(), transcript(S1), ALPHA.into(), None);
        let new = store.read_conversation(S2.into(), transcript(S2), ALPHA.into(), None);
        assert!(new > old);
        let seen = until_conversation(&events, new);
        let conversations: Vec<(u64, &str)> = seen.iter().filter(|e| matches!(e, StoreEvent::Conversation { .. })).map(|e| { let (seq, id, _) = conversation_of(e); (seq, id) }).collect();
        assert_eq!(conversations.last(), Some(&(new, S2)));
        assert!(conversations[..conversations.len() - 1].iter().all(|(seq, _)| *seq == old), "{conversations:?}");
    }

    #[test]
    fn history_prompts_are_served_for_a_history_session() {
        const S4: &str = "44444444-aaaa-4bbb-8ccc-000000000004";
        let dir = tempfile::tempdir().unwrap();
        let (store, events) = spawn(fixture_claude(), dir.path().join("index.sqlite"));

        let matcher = Matcher::words("flaky").unwrap();
        let seq = store.read_history(S4.into(), "-Users-me-Code-alpha".into(), Some(matcher.clone()));
        let seen = until_conversation(&events, seq);
        let (_, session_id, view) = conversation_of(seen.last().unwrap());
        let view = view.as_ref().unwrap();
        assert_eq!(session_id, S4);
        assert_eq!(view.cards.len(), 4, "one card per prompt of the session in history.jsonl");
        let Card::Turn(second) = &view.cards[1] else { panic!("a prompt is a turn card") };
        assert_eq!(second.label, "TURN 2");
        assert!(second.prompt.text.contains("Fix the flaky test") && second.prompt.text.contains("line1\nline2\nline3"), "pasted text restored: {:?}", second.prompt.text);
        assert_eq!((view.pattern.as_deref(), view.hits.len()), (Some(matcher.pattern()), 1), "marked like a transcript");

        let seq = store.read_history("history:-Users-me-Code-beta:1748736000000".into(), "-Users-me-Code-beta".into(), None);
        let seen = until_conversation(&events, seq);
        assert_eq!(conversation_of(seen.last().unwrap()).2.as_ref().unwrap().cards.len(), 2, "an anonymous session is its time group");
    }

    /// Events up to and including the first `Exported`.
    fn until_exported(events: &Receiver<StoreEvent>) -> (PathBuf, Result<usize, String>) {
        loop {
            if let StoreEvent::Exported { dest, result } = events.recv_blocking().expect("store threads ended early") {
                return (dest, result);
            }
        }
    }

    #[test]
    fn an_export_writes_the_file_and_reports_skipped_lines() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join(format!("{S1}.jsonl"));
        let mut text = std::fs::read_to_string(transcript(S1)).unwrap();
        text.push_str("not json\n");
        std::fs::write(&src, text).unwrap();
        let (store, events) = spawn(fixture_claude(), dir.path().join("index.sqlite"));

        let dest = dir.path().join("out").join("alpha-11111111.txt");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        store.export(src.clone(), dest.clone());
        let expected = agent_session_core::transcript::read_transcript(&src).unwrap();
        assert!(expected.skipped_lines >= 1);
        assert_eq!(until_exported(&events), (dest.clone(), Ok(expected.skipped_lines)));
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), expected.text, "what transcript-reader writes");

        store.export(dir.path().join("gone.jsonl"), dir.path().join("gone.txt"));
        let (_, result) = until_exported(&events);
        assert!(result.is_err_and(|e| e.contains("gone.jsonl")));
    }

    #[test]
    fn an_export_is_not_superseded_by_a_conversation_request() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("s1.txt");
        let (requests, queue) = mpsc::channel();
        let (event_tx, events) = async_channel::unbounded();
        let conversation = |seq, id| ReadRequest::Conversation { seq, session_id: String::from(id), path: transcript(id), project_path: ALPHA.into(), matcher: None };
        requests.send(conversation(1, S1)).unwrap();
        requests.send(ReadRequest::Export { src: transcript(S1), dest: dest.clone() }).unwrap();
        requests.send(conversation(2, S2)).unwrap();
        requests.send(conversation(3, S1)).unwrap();
        drop(requests);
        serve_reads(queue, Arc::new(AtomicU64::new(3)), event_tx, fixture_claude());

        let served: Vec<StoreEvent> = std::iter::from_fn(|| events.try_recv().ok()).collect();
        assert!(matches!(&served[..], [StoreEvent::Exported { result: Ok(_), .. }, StoreEvent::Conversation { seq: 3, .. }]), "{served:?}");
        assert!(dest.is_file());
    }

    #[test]
    fn a_missing_transcript_is_reported_instead_of_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let (store, events) = spawn(fixture_claude(), dir.path().join("index.sqlite"));
        let gone = dir.path().join("gone.jsonl");
        let seq = store.read_conversation(S1.into(), gone, ALPHA.into(), None);
        let seen = until_conversation(&events, seq);
        let Err(message) = conversation_of(seen.last().unwrap()).2 else { panic!("a missing file cannot be read") };
        assert!(message.contains("gone.jsonl"), "{message}");

        let seq = store.read_conversation(S1.into(), transcript(S1), ALPHA.into(), None);
        let seen = until_conversation(&events, seq);
        assert!(conversation_of(seen.last().unwrap()).2.is_ok(), "the reader thread is still there");
    }

    /// A copy of the fixture tree that a test may append to or delete from.
    fn fixture_copy(dir: &std::path::Path) -> ClaudeDir {
        let claude = dir.join("claude");
        crate::keyboard_tests::copy_fixture(fixture_claude().root(), &claude);
        ClaudeDir::new(claude)
    }

    fn usage_events(events: &[StoreEvent]) -> Vec<(bool, Vec<&str>)> {
        events
            .iter()
            .filter_map(|e| match e {
                StoreEvent::Usage { sessions, full } => Some((*full, sessions.iter().map(|s| s.session.id.as_str()).collect())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn usage_follows_the_first_load_and_later_scans_send_only_changed_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let claude = fixture_copy(dir.path());
        let (store, events) = spawn(claude.clone(), dir.path().join("index.sqlite"));
        let seen = until_settled(&events);
        assert!(matches!(&seen[..2], [StoreEvent::Loaded { .. }, StoreEvent::Usage { full: true, .. }]), "the ledger right after the index: {seen:?}");
        let scanned = usage_events(&seen);
        assert_eq!(scanned.len(), 2, "the full load, then what the scan wrote: {scanned:?}");
        assert!(!scanned[1].0 && scanned[1].1.contains(&S1), "the cold scan wrote S1's calls: {scanned:?}");
        let responses = |events: &[StoreEvent]| -> u64 {
            let Some(StoreEvent::Usage { sessions, .. }) = events.iter().rfind(|e| matches!(e, StoreEvent::Usage { .. })) else { panic!("no usage in {events:?}") };
            sessions.iter().filter(|s| s.session.id == S1).flat_map(|s| &s.hours).map(|h| h.responses).sum()
        };
        let before = responses(&seen);
        assert!(before > 0);
        let done = seen.iter().position(|e| matches!(e, StoreEvent::ScanDone { .. })).unwrap();
        let usage = seen.iter().rposition(|e| matches!(e, StoreEvent::Usage { .. })).unwrap();
        assert!(usage < done, "the usage of a scan arrives before its ScanDone");

        crate::keyboard_tests::append_response(claude.root(), S1, "msg_appended", "claude-opus-5", chrono::Utc::now().timestamp_millis());
        store.request(StoreRequest::Scan { quiet: false });
        let seen = until_settled(&events);
        assert_eq!(usage_events(&seen), [(false, vec![S1])], "only the session the scan changed");
        assert_eq!(responses(&seen), before + 1, "S1's calls and the appended one");

        store.request(StoreRequest::Scan { quiet: false });
        assert_eq!(usage_events(&until_settled(&events)), [], "a scan that changed nothing sends no usage");
    }

    #[test]
    fn a_rebuild_does_not_drop_usage_the_ui_already_has() {
        let dir = tempfile::tempdir().unwrap();
        let claude = fixture_copy(dir.path());
        let (store, events) = spawn(claude.clone(), dir.path().join("index.sqlite"));
        let mut usage = agent_session_core::usage::UsageData::default();
        let take = |seen: &[StoreEvent], usage: &mut agent_session_core::usage::UsageData| {
            for ev in seen {
                if let StoreEvent::Usage { sessions, full } = ev {
                    if *full { usage.replace_all(sessions.clone()) } else { usage.apply(sessions.clone()) }
                }
            }
        };
        take(&until_settled(&events), &mut usage);
        let before = usage.sessions.get(S2).cloned().expect("S2 has calls");
        assert!(!before.hours.is_empty());

        // S2 is not in history.jsonl either: after the rebuild the index knows nothing of it
        std::fs::remove_file(claude.root().join("projects/-Users-me-Code-alpha").join(format!("{S2}.jsonl"))).unwrap();
        store.request(StoreRequest::Scan { quiet: false });
        take(&until_settled(&events), &mut usage);
        store.request(StoreRequest::Rebuild);
        let seen = until_settled(&events);
        assert!(!loaded_counts(&seen).is_empty() && !seen.iter().any(|e| matches!(e, StoreEvent::Session(s) if s.id == S2)), "gone from the table");
        let sent = usage_events(&seen);
        assert!(!sent.is_empty() && sent.iter().all(|(full, ids)| !full && !ids.contains(&S2)), "a rebuild only sends what it read again: {sent:?}");
        take(&seen, &mut usage);
        assert_eq!(usage.sessions.get(S2), Some(&before), "the UI keeps S2's usage");
    }

    /// A worker on a fixture copy whose index holds the fixture already, without the start-up
    /// thread: the test sends its requests through `serve` and reads every event.
    fn scanned_worker(dir: &std::path::Path) -> (Worker, ClaudeDir, Receiver<StoreEvent>) {
        let (tx, events) = async_channel::unbounded();
        let favorites = FavoritesDir { root: dir.join("favorites"), trash: favorites::Trash::Dir(dir.join("trash")) };
        let claude = fixture_copy(dir);
        let mut worker = Worker::open(claude.clone(), &dir.join("index.sqlite"), favorites, tx).unwrap();
        worker.serve(StoreRequest::Scan { quiet: false });
        let seen: Vec<_> = std::iter::from_fn(|| events.try_recv().ok()).collect();
        assert!(seen.iter().any(|e| matches!(e, StoreEvent::ScanDone { .. })), "{seen:?}");
        (worker, claude, events)
    }

    fn drain(events: &Receiver<StoreEvent>) -> Vec<StoreEvent> {
        std::iter::from_fn(|| events.try_recv().ok()).collect()
    }

    #[test]
    fn a_quiet_scan_that_finds_the_index_busy_is_skipped_without_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let (mut worker, claude, events) = scanned_worker(dir.path());
        worker.db.set_busy_timeout(Duration::from_millis(100)).unwrap();
        // something to index, and asm scanning the same index
        crate::keyboard_tests::append_response(claude.root(), S1, "msg_busy", "claude-opus-5", chrono::Utc::now().timestamp_millis());
        let writer = Db::open(&dir.path().join("index.sqlite")).unwrap();
        writer.conn().execute_batch("BEGIN IMMEDIATE").unwrap();

        worker.serve(StoreRequest::Scan { quiet: true });
        let seen = drain(&events);
        assert!(matches!(&seen[..], [StoreEvent::ScanSkipped]), "no progress, no failure, nothing at all: {seen:?}");

        worker.serve(StoreRequest::Scan { quiet: false });
        let seen = drain(&events);
        assert!(seen.iter().any(|e| matches!(e, StoreEvent::Progress(_))), "a scan the user asked for shows its progress: {seen:?}");
        assert!(matches!(seen.last(), Some(StoreEvent::Failed(m)) if m.contains("locked")), "and says why it failed: {seen:?}");

        writer.conn().execute_batch("COMMIT").unwrap();
        worker.serve(StoreRequest::Scan { quiet: true });
        let seen = drain(&events);
        assert!(!seen.iter().any(|e| matches!(e, StoreEvent::Progress(_))), "a quiet scan shows no progress: {seen:?}");
        let session = seen.iter().position(|e| matches!(e, StoreEvent::Session(s) if s.id == S1));
        let usage = seen.iter().position(|e| matches!(e, StoreEvent::Usage { full: false, .. }));
        let done = seen.iter().position(|e| matches!(e, StoreEvent::ScanDone { quiet: true, .. }));
        assert!(session.is_some() && session < usage && usage < done, "what it wrote, its usage, then ScanDone: {seen:?}");
    }

    #[test]
    fn a_quiet_scan_that_finds_the_ledger_busy_changes_nothing_on_screen() {
        let dir = tempfile::tempdir().unwrap();
        let (mut worker, claude, events) = scanned_worker(dir.path());
        crate::keyboard_tests::append_response(claude.root(), S1, "msg_ledger_busy", "claude-opus-5", chrono::Utc::now().timestamp_millis());
        // another process writing the ledger: the scan parses and writes the index, then cannot
        // write the ledger before its commit and rolls back
        let other = tempfile::tempdir().unwrap();
        let writer = Db::open(&other.path().join("index.sqlite")).unwrap();
        let ledger = agent_session_core::claude::paths::ledger_path(&dir.path().join("index.sqlite"));
        writer.conn().execute_batch(&format!("ATTACH '{}' AS l; BEGIN; INSERT INTO l.meta (key, value) VALUES ('test-lock', '1');", ledger.display())).unwrap();

        worker.serve(StoreRequest::Scan { quiet: true });
        let seen = drain(&events);
        assert!(matches!(&seen[..], [StoreEvent::ScanSkipped]), "rows of a scan that rolled back never reach the table: {seen:?}");

        writer.conn().execute_batch("ROLLBACK").unwrap();
        worker.serve(StoreRequest::Scan { quiet: true });
        let seen = drain(&events);
        assert!(seen.iter().any(|e| matches!(e, StoreEvent::Session(s) if s.id == S1)), "the next one writes it: {seen:?}");
        assert!(seen.iter().any(|e| matches!(e, StoreEvent::ScanDone { quiet: true, .. })), "{seen:?}");
    }

    #[test]
    fn an_unusable_index_path_is_reported_instead_of_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        let (_store, events) = spawn(fixture_claude(), blocker.join("index.sqlite"));
        assert!(matches!(events.recv_blocking().unwrap(), StoreEvent::Failed(_)));
    }

    #[test]
    fn requests_after_the_index_failed_to_open_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        let (store, events) = spawn(fixture_claude(), blocker.join("index.sqlite"));
        assert!(matches!(events.recv_blocking().unwrap(), StoreEvent::Failed(m) if m.starts_with("open index")));
        // the thread is gone by the time its failure arrives: nothing would ever answer a scan
        assert!(!store.request(StoreRequest::Scan { quiet: true }), "the app must hear that the scan never started");
        assert!(!store.request(StoreRequest::Scan { quiet: false }));
    }
}
