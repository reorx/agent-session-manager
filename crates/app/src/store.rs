//! Bridge to `crates/core`: one background thread owns the SQLite index (a `Db` never leaves its
//! thread), serves requests from the UI and streams `StoreEvent`s back over an async channel.
//!
//! Start-up sequence: open the index → `Loaded` with whatever is already indexed (hot start) →
//! incremental scan, streaming `Progress` and every written `Session` (cold start fills the
//! table this way) → `ScanDone` → a second `Loaded` when the scan changed anything, so rows that
//! were streamed before their project's path was known end up exact.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agent_session_core::claude::paths::ClaudeDir;
use agent_session_core::index::{Db, IndexStats, Indexer, ListFilter, Order, Progress, ScanEvent, ScanReport};
use agent_session_core::model::{MessageRow, SessionSummary};
use anyhow::Result;
use async_channel::{Receiver, Sender};

pub enum StoreRequest {
    Scan,
    Rebuild,
    Messages(String),
}

#[derive(Debug)]
pub enum StoreEvent {
    /// Everything in the index, replacing whatever the UI holds.
    Loaded { sessions: Vec<SessionSummary>, elapsed: Duration },
    Progress(Progress),
    /// One session written by the running scan (new or changed).
    Session(SessionSummary),
    ScanDone { report: ScanReport, stats: IndexStats },
    Messages { session_id: String, messages: Vec<MessageRow> },
    Failed(String),
}

pub struct Store {
    requests: mpsc::Sender<StoreRequest>,
}

impl Store {
    pub fn spawn(claude: ClaudeDir, db_path: PathBuf) -> (Store, Receiver<StoreEvent>) {
        let (requests, request_rx) = mpsc::channel();
        let (event_tx, events) = async_channel::unbounded();
        std::thread::Builder::new()
            .name("asm-store".into())
            .spawn(move || run(claude, db_path, request_rx, event_tx))
            .expect("failed to spawn the store thread");
        (Store { requests }, events)
    }

    pub fn request(&self, request: StoreRequest) {
        self.requests.send(request).ok();
    }
}

/// Thread body: every failure becomes a `Failed` event instead of taking the thread down.
fn run(claude: ClaudeDir, db_path: PathBuf, requests: mpsc::Receiver<StoreRequest>, events: Sender<StoreEvent>) {
    let mut worker = match Worker::open(claude, &db_path, events.clone()) {
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
            StoreRequest::Scan => worker.scan(false),
            StoreRequest::Rebuild => worker.scan(true),
            StoreRequest::Messages(id) => worker.messages(id),
        };
        if let Err(e) = result {
            events.send_blocking(StoreEvent::Failed(format!("{e:#}"))).ok();
        }
    }
}

struct Worker {
    db: Db,
    indexer: Indexer,
    events: Sender<StoreEvent>,
}

impl Worker {
    fn open(claude: ClaudeDir, db_path: &std::path::Path, events: Sender<StoreEvent>) -> Result<Worker> {
        Ok(Worker { db: Db::open(db_path)?, indexer: Indexer::new(claude), events })
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
        let report = self.indexer.scan_streaming(&mut self.db, &mut |ev| {
            let event = match ev {
                ScanEvent::Progress(p) => StoreEvent::Progress(p),
                ScanEvent::Session(s) => StoreEvent::Session(s),
            };
            events.send_blocking(event).ok();
        })?;
        let changed = report.files_seen != report.files_unchanged || report.files_missing > 0 || report.history_sessions > 0;
        self.events.send_blocking(StoreEvent::ScanDone { report, stats: self.db.stats()? })?;
        if changed {
            self.load()?;
        }
        Ok(())
    }

    fn messages(&self, session_id: String) -> Result<()> {
        let messages = self.db.messages(&session_id)?;
        self.events.send_blocking(StoreEvent::Messages { session_id, messages })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S1: &str = "11111111-aaaa-4bbb-8ccc-000000000001";

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
        let (_store, events) = Store::spawn(fixture_claude(), dir.path().join("index.sqlite"));
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
            let (_store, events) = Store::spawn(fixture_claude(), db_path.clone());
            until_settled(&events);
        }
        let (_store, events) = Store::spawn(fixture_claude(), db_path);
        let seen = until_settled(&events);
        assert_eq!(loaded_counts(&seen), [8], "one load, no reload");
        assert_eq!(streamed(&seen), 0);
        let Some(StoreEvent::Loaded { sessions, .. }) = seen.first() else { panic!("first event must be Loaded") };
        assert!(sessions.windows(2).all(|w| w[0].updated_at >= w[1].updated_at), "newest first");
    }

    #[test]
    fn messages_of_a_session_are_served_on_request_and_a_rebuild_streams_everything_again() {
        let dir = tempfile::tempdir().unwrap();
        let (store, events) = Store::spawn(fixture_claude(), dir.path().join("index.sqlite"));
        until_settled(&events);

        store.request(StoreRequest::Messages(S1.into()));
        let StoreEvent::Messages { session_id, messages } = events.recv_blocking().unwrap() else { panic!("expected Messages") };
        assert_eq!(session_id, S1);
        assert_eq!(messages.len(), 10);

        store.request(StoreRequest::Rebuild);
        let seen = until_settled(&events);
        assert_eq!(loaded_counts(&seen), [0, 8], "the table empties, then fills again");
        assert_eq!(streamed(&seen), 8);
    }

    #[test]
    fn an_unusable_index_path_is_reported_instead_of_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        let (_store, events) = Store::spawn(fixture_claude(), blocker.join("index.sqlite"));
        assert!(matches!(events.recv_blocking().unwrap(), StoreEvent::Failed(_)));
    }
}
