//! Bridge to `crates/core`: one background thread owns the SQLite index (a `Db` never leaves its
//! thread), serves requests from the UI and streams `StoreEvent`s back over an async channel.
//!
//! Start-up sequence: open the index → `Loaded` with whatever is already indexed (hot start) →
//! incremental scan, streaming `Progress` and every written `Session` (cold start fills the
//! table this way) → `ScanDone` → a second `Loaded` when the scan changed anything, so rows that
//! were streamed before their project's path was known end up exact.
//!
//! A second thread, `asm-reader`, reads transcripts for the detail panel straight from the file
//! (no `Db`), so a conversation shows up while a scan holds the store thread. Its requests
//! supersede each other like searches do (`seq`, see `search.rs`), and its `Conversation` events
//! go out on the same channel.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use agent_session_core::claude::conversation::read_conversation;
use agent_session_core::claude::paths::ClaudeDir;
use agent_session_core::index::{Db, IndexStats, Indexer, ListFilter, Order, Progress, ScanEvent, ScanReport};
use agent_session_core::model::SessionSummary;
use agent_session_core::search::Matcher;
use anyhow::{Result, anyhow, ensure};
use agent_session_core::{config, favorites::{self, FavoritesDir}};
use async_channel::{Receiver, Sender};
use chrono::Local;

use crate::conversation::{self, ConversationView};

pub enum StoreRequest {
    Favorite(String),
    Unfavorite(String),
    SetFavoritesDir(Option<PathBuf>),
    Scan,
    Rebuild,
}

#[derive(Debug)]
pub enum StoreEvent {
    /// Everything in the index, replacing whatever the UI holds.
    Loaded { sessions: Vec<SessionSummary>, elapsed: Duration },
    Progress(Progress),
    /// One session written by the running scan (new or changed).
    Session(SessionSummary),
    ScanDone { report: ScanReport, stats: IndexStats },
    /// A transcript read for the detail panel (`Store::read_conversation`), already built into
    /// cards. `read` is the file read, `build` the conversion; both ran on the reader thread.
    Conversation { seq: u64, session_id: String, conversation: Result<Arc<ConversationView>, String>, read: Duration, build: Duration },
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

struct ReadRequest {
    seq: u64,
    session_id: String,
    path: PathBuf,
    project_path: String,
    matcher: Option<Matcher>,
}

impl Store {
    pub fn spawn(claude: ClaudeDir, db_path: PathBuf, favorites: FavoritesDir) -> (Store, Receiver<StoreEvent>) {
        let (requests, request_rx) = mpsc::channel();
        let (event_tx, events) = async_channel::unbounded();
        let (reads, read_rx) = mpsc::channel();
        let latest = Arc::new(AtomicU64::new(0));
        let (reader_latest, reader_events) = (latest.clone(), event_tx.clone());
        std::thread::Builder::new()
            .name("asm-store".into())
            .spawn(move || run(claude, db_path, favorites, request_rx, event_tx))
            .expect("failed to spawn the store thread");
        std::thread::Builder::new()
            .name("asm-reader".into())
            .spawn(move || serve_reads(read_rx, reader_latest, reader_events))
            .expect("failed to spawn the reader thread");
        (Store { requests, reads, latest }, events)
    }

    pub fn request(&self, request: StoreRequest) {
        self.requests.send(request).ok();
    }

    /// Read a transcript for the detail panel on the reader thread, superseding any read that
    /// has not started yet. Returns the `seq` the `Conversation` event carries; events of older
    /// requests are stale from now on.
    pub fn read_conversation(&self, session_id: String, path: PathBuf, project_path: String, matcher: Option<Matcher>) -> u64 {
        let seq = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        self.reads.send(ReadRequest { seq, session_id, path, project_path, matcher }).ok();
        seq
    }
}

/// Reader thread body: only the newest request is read; whatever queued up before it, or was
/// superseded while being read, is dropped. Never touches the index, so a scan never delays it.
fn serve_reads(requests: mpsc::Receiver<ReadRequest>, latest: Arc<AtomicU64>, events: Sender<StoreEvent>) {
    let stale = |seq: u64| latest.load(Ordering::SeqCst) != seq;
    while let Ok(mut request) = requests.recv() {
        while let Ok(newer) = requests.try_recv() {
            request = newer;
        }
        if stale(request.seq) {
            continue;
        }
        let seq = request.seq;
        let event = read(request);
        if !stale(seq) {
            events.send_blocking(event).ok();
        }
    }
}

fn read(request: ReadRequest) -> StoreEvent {
    let started = Instant::now();
    let conversation = read_conversation(&request.path, request.matcher.as_ref());
    let read = started.elapsed();
    let conversation = conversation
        .map(|conv| {
            let mut view = conversation::build(&conv, &request.project_path, Local::now().date_naive());
            view.pattern = request.matcher.as_ref().map(|m| m.pattern().to_string());
            Arc::new(view)
        })
        .map_err(|e| format!("{e:#}"));
    StoreEvent::Conversation { seq: request.seq, session_id: request.session_id, conversation, read, build: started.elapsed() - read }
}

/// Thread body: every failure becomes a `Failed` event instead of taking the thread down.
fn run(claude: ClaudeDir, db_path: PathBuf, favorites: FavoritesDir, requests: mpsc::Receiver<StoreRequest>, events: Sender<StoreEvent>) {
    let mut worker = match Worker::open(claude, &db_path, favorites, events.clone()) {
        Ok(worker) => worker,
        Err(e) => {
            events.send_blocking(StoreEvent::Failed(format!("open index: {e:#}"))).ok();
            return;
        }
    };
    if let Err(e) = worker.load().and_then(|_| worker.scan(false)) {
        events.send_blocking(StoreEvent::Failed(format!("{e:#}"))).ok();
    }
    for request in requests {
        let result = match request {
            StoreRequest::Favorite(id) => worker.favorite(id, false),
            StoreRequest::Unfavorite(id) => worker.favorite(id, true),
            StoreRequest::SetFavoritesDir(dir) => worker.set_favorites_dir(dir),
            StoreRequest::Scan => worker.scan(false),
            StoreRequest::Rebuild => worker.scan(true),
        };
        if let Err(e) = result {
            events.send_blocking(StoreEvent::Failed(format!("{e:#}"))).ok();
        }
    }
}

struct Worker {
    claude: ClaudeDir,
    favorites: FavoritesDir,
    db: Db,
    indexer: Indexer,
    events: Sender<StoreEvent>,
}

impl Worker {
    fn open(claude: ClaudeDir, db_path: &std::path::Path, favorites: FavoritesDir, events: Sender<StoreEvent>) -> Result<Worker> {
        Ok(Worker { db: Db::open(db_path)?, indexer: Indexer::new(claude.clone(), Some(favorites.claude())), claude, favorites, events })
    }

    fn favorite(&mut self, id: String, remove: bool) -> Result<()> {
        let s = self.db.session(&id)?.ok_or_else(|| anyhow!("session not found"))?;
        if remove { favorites::remove(&self.favorites.root, &s.project_key, &id, &self.favorites.trash)?; }
        else {
            ensure!(!s.history_only(), "cannot favorite a history-only session");
            favorites::save(&self.favorites.root, s.file_path.as_ref().ok_or_else(|| anyhow!("no transcript"))?, &s.project_key, &id)?;
        }
        self.scan(false)?;
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
        self.scan(false)?;
        self.events.send_blocking(StoreEvent::FavoritesDirChanged { dir: new, moved: report.moved, skipped: report.skipped })?; Ok(())
    }

    fn load(&self) -> Result<()> {
        let t = Instant::now();
        let sessions = self.db.list_sessions(&ListFilter { order: Order::UpdatedDesc, ..Default::default() })?;
        self.events.send_blocking(StoreEvent::Loaded { sessions, elapsed: t.elapsed() })?;
        Ok(())
    }

    fn scan(&mut self, rebuild: bool) -> Result<()> {
        if rebuild {
            self.db.clear_all()?;
            self.load()?;
        }
        let events = self.events.clone();
        let result = self.indexer.scan_streaming(&mut self.db, &mut |ev| {
            let event = match ev {
                ScanEvent::Progress(p) => StoreEvent::Progress(p),
                ScanEvent::Session(s) => StoreEvent::Session(s),
            };
            events.send_blocking(event).ok();
        });
        let report = match result {
            Err(e) if e.is::<agent_session_core::index::indexer::RootsChanged>() => {
                self.events.send_blocking(StoreEvent::RebuildingRoots)?;
                return self.scan(true);
            }
            result => result?,
        };
        let changed = report.files_seen != report.files_unchanged || report.files_missing > 0 || report.history_sessions > 0;
        self.events.send_blocking(StoreEvent::ScanDone { report, stats: self.db.stats()? })?;
        if changed {
            self.load()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S1: &str = "11111111-aaaa-4bbb-8ccc-000000000001";

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
        assert_eq!((view.summary.as_ref(), view.cards.len()), ("4 turns · 3 tool calls", 4), "read from the transcript, built into cards");
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
            requests.send(ReadRequest { seq, session_id: id.into(), path: transcript(id), project_path: ALPHA.into(), matcher: None }).unwrap();
        }
        drop(requests);
        serve_reads(queue, latest, event_tx);
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

    #[test]
    fn an_unusable_index_path_is_reported_instead_of_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        let (_store, events) = spawn(fixture_claude(), blocker.join("index.sqlite"));
        assert!(matches!(events.recv_blocking().unwrap(), StoreEvent::Failed(_)));
    }
}
