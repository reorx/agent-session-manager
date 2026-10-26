//! Incremental indexer: stat every transcript, parse only what changed (in parallel), write in
//! one transaction, then fill gaps from `history.jsonl`. See `kb/docs/data-layer-design.md` §4.
//!
//! Subagent transcripts go through the same incremental bookkeeping, but only their API usage is
//! read: it counts towards the cost of the session they belong to.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use rayon::prelude::*;

use super::db::{Db, IndexState};
use crate::claude::discover::{TranscriptFile, discover_all, subagent_count};
use crate::claude::history::{history_sessions, load_history};
use crate::claude::parse::{SessionDelta, TranscriptParser, map_file};
use crate::claude::paths::ClaudeDir;
use crate::model::{MessageRow, SessionSummary, Source, UsageRow};

pub struct Indexer {
    claude: ClaudeDir,
    favorites: Option<ClaudeDir>,
}

/// Scan progress, reported after every file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub phase: &'static str,
}

/// What a scan reports while it runs: progress ticks, plus every session row it writes (read
/// back from the index, so it equals `Db::session`). The UI fills its table from these.
#[derive(Clone, Debug, PartialEq)]
pub enum ScanEvent {
    Progress(Progress),
    Session(SessionSummary),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Session transcripts; the `files_*` counters below are about these only.
    pub files_seen: usize,
    /// New files parsed from the start.
    pub files_parsed: usize,
    pub files_appended: usize,
    pub files_rebuilt: usize,
    pub files_unchanged: usize,
    pub files_missing: usize,
    pub files_superseded: usize,
    pub bytes_parsed: u64,
    /// Sessions written from history.jsonl (new or refreshed).
    pub history_sessions: usize,
    /// Subagent transcripts seen, and parsed for their usage (new, appended or rewritten).
    pub subagents_seen: usize,
    pub subagents_parsed: usize,
    pub elapsed: Duration,
    pub elapsed_parse: Duration,
    pub elapsed_write: Duration,
    pub elapsed_history: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    Unchanged,
    /// Parse from the given offset.
    Append(u64),
    /// Parse from 0, replacing whatever was indexed.
    Full,
    /// File shrank or was rewritten: drop the old rows first.
    Rebuild,
}

struct Parsed {
    file: TranscriptFile,
    plan: Plan,
    delta: SessionDelta,
    messages: Vec<MessageRow>,
    usage: Vec<UsageRow>,
    /// Absolute offset up to which the file has been parsed.
    indexed_offset: u64,
    bytes_parsed: u64,
    subagent_count: u32,
}

impl Indexer {
    pub fn new(claude: ClaudeDir, favorites: Option<ClaudeDir>) -> Self {
        Indexer { claude, favorites }
    }

    /// Bring the index up to date with the files on disk.
    pub fn scan(&self, db: &mut Db, progress: &mut dyn FnMut(&Progress)) -> Result<ScanReport> {
        self.scan_streaming(db, &mut |ev| {
            if let ScanEvent::Progress(p) = ev {
                progress(&p);
            }
        })
    }

    /// Like `scan`, but also hands out each session row as soon as it is written.
    pub fn scan_streaming(&self, db: &mut Db, emit: &mut dyn FnMut(ScanEvent)) -> Result<ScanReport> {
        let started = Instant::now();
        let mut report = ScanReport::default();
        let claude_root = crate::config::canonical_root(self.claude.root())?.to_string_lossy().into_owned();
        let favorites_root = self.favorites.as_ref().map(|f| crate::config::canonical_root(f.root())).transpose()?.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        for (key, value) in [("claude_root", &claude_root), ("favorites_root", &favorites_root)] {
            if db.meta(key)?.is_some_and(|previous| previous != *value) { return Err(RootsChanged.into()); }
        }
        let discovered = discover_all(&self.claude, self.favorites.as_ref())?;
        let files = discovered.sessions; let subagents = discovered.subagents;
        let live_ids: HashSet<String> = files.iter().map(|f| f.session_id.clone()).collect();
        let on_disk: HashSet<PathBuf> = files.iter().chain(&subagents).map(|f| f.path.clone()).collect();
        report.files_seen = files.len();
        report.subagents_seen = subagents.len();
        let states = db.all_index_states()?;
        let gone = Gone::find(&states, &on_disk);
        let superseded: HashSet<_> = gone.sessions.iter().filter(|(_, s)| live_ids.contains(&s.session_id)).map(|(_, s)| s.session_id.clone()).collect();
        let planned: Vec<(TranscriptFile, Plan)> = files
            .into_iter()
            .chain(subagents)
            .map(|f| {
                let plan = if superseded.contains(&f.session_id) { Plan::Full } else { plan_for(&f, states.get(&f.path)) };
                (f, plan)
            })
            .collect();
        report.files_unchanged = planned.iter().filter(|(f, p)| f.agent.is_empty() && *p == Plan::Unchanged).count();
        emit(ScanEvent::Progress(Progress { done: report.files_unchanged, total: report.files_seen, phase: "scanning" }));

        let to_parse: Vec<(TranscriptFile, Plan)> = planned.into_iter().filter(|(_, p)| *p != Plan::Unchanged).collect();
        let t = Instant::now();
        let parsed: Vec<Parsed> = to_parse.into_par_iter().map(|(f, plan)| parse_file(f, plan)).collect::<Result<Vec<_>>>()?;
        report.elapsed_parse = t.elapsed();
        let (sessions, subagents): (Vec<&Parsed>, Vec<&Parsed>) = parsed.iter().partition(|p| p.file.agent.is_empty());

        let mut done = report.files_unchanged;
        let t = Instant::now();
        db.transaction(|db| {
            db.set_meta("claude_root", &claude_root)?;
            db.set_meta("favorites_root", &favorites_root)?;
            for (path, state) in &gone.sessions {
                if live_ids.contains(&state.session_id) { db.delete_index_state(path)?; report.files_superseded += 1; }
            }
            // Subagent usage first, so every session row written below already counts it.
            // Sessions whose subagents changed but whose own transcript did not are sent at the end.
            let mut touched: BTreeSet<String> = forget_subagents(db, &gone.subagents, &live_ids)?;
            for p in &subagents {
                write_subagent(db, p)?;
                touched.insert(p.file.session_id.clone());
                report.subagents_parsed += 1;
                report.bytes_parsed += p.bytes_parsed;
            }
            for p in &sessions {
                write_parsed(db, p)?;
                emit_session(db, &p.file.session_id, emit)?;
                touched.remove(&p.file.session_id);
                match p.plan {
                    Plan::Full => report.files_parsed += 1,
                    Plan::Append(_) => report.files_appended += 1,
                    Plan::Rebuild => report.files_rebuilt += 1,
                    Plan::Unchanged => {}
                }
                report.bytes_parsed += p.bytes_parsed;
                done += 1;
                emit(ScanEvent::Progress(Progress { done, total: report.files_seen, phase: "indexing" }));
            }
            let missing = mark_missing(db, &gone.sessions.iter().copied().filter(|(_, s)| !live_ids.contains(&s.session_id)).collect::<Vec<_>>())?;
            report.files_missing = missing.len();
            for id in &missing {
                if touched.remove(id) {
                    db.refresh_session_usage(id)?;
                }
                emit_session(db, id, emit)?;
            }
            for id in &touched {
                db.refresh_session_usage(id)?;
                emit_session(db, id, emit)?;
            }
            report.elapsed_write = t.elapsed();
            let t = Instant::now();
            report.history_sessions = self.import_history(db, emit)?;
            report.elapsed_history = t.elapsed();
            Ok(())
        })?;
        emit(ScanEvent::Progress(Progress { done: report.files_seen, total: report.files_seen, phase: "done" }));
        report.elapsed = started.elapsed();
        Ok(report)
    }

    /// Drop the whole index and scan again.
    pub fn rebuild(&self, db: &mut Db, progress: &mut dyn FnMut(&Progress)) -> Result<ScanReport> {
        db.clear_all()?;
        self.scan(db, progress)
    }

    /// Sessions that exist only in history.jsonl (transcript gone or never indexed). Rows that
    /// are already indexed unchanged are left alone, so rescans stay quiet.
    fn import_history(&self, db: &Db, emit: &mut dyn FnMut(ScanEvent)) -> Result<usize> {
        let by_project = load_history(&self.claude.history_file())?;
        let cache = self.claude.paste_cache_dir();
        let sources = db.session_sources()?;
        let mut written = 0;
        for (key, rows) in &by_project {
            let project_id = db.upsert_project(key, rows.first().map(|r| r.project.as_str()))?;
            for s in history_sessions(key, rows, &cache) {
                if sources.get(&s.id) == Some(&Source::Transcript) {
                    continue;
                }
                if sources.contains_key(&s.id) && db.session(&s.id)?.as_ref() == Some(&s) {
                    continue;
                }
                db.upsert_session(&s, project_id)?;
                emit_session(db, &s.id, emit)?;
                written += 1;
            }
        }
        Ok(written)
    }
}

fn plan_for(f: &TranscriptFile, state: Option<&IndexState>) -> Plan {
    let Some(state) = state else { return Plan::Full };
    if f.size == state.size && f.mtime_ns == state.mtime_ns {
        return Plan::Unchanged;
    }
    if f.size < state.indexed_offset || (state.inode.is_some() && state.inode != Some(f.inode)) {
        return Plan::Rebuild;
    }
    Plan::Append(state.indexed_offset)
}

fn parse_file(file: TranscriptFile, plan: Plan) -> Result<Parsed> {
    let handle = File::open(&file.path)?;
    let len = handle.metadata()?.len().min(file.size);
    let bytes = map_file(&handle, len)?;
    let bytes = &bytes[..len as usize];
    let (plan, from) = match plan {
        Plan::Append(off) if off == 0 || bytes.get(off as usize - 1) == Some(&b'\n') => (Plan::Append(off), off),
        Plan::Append(_) => (Plan::Rebuild, 0),
        other => (other, 0),
    };
    let is_session = file.agent.is_empty();
    let mut parser = if is_session { TranscriptParser::new(&file.session_id) } else { TranscriptParser::usage_only() };
    let feed = parser.feed(&bytes[from as usize..], from);
    let subagents = if is_session { subagent_count(&file.path) } else { 0 };
    Ok(Parsed {
        plan,
        delta: parser.into_delta(),
        messages: feed.messages,
        usage: feed.usage,
        indexed_offset: from + feed.consumed,
        bytes_parsed: len - from,
        subagent_count: subagents,
        file: file.clone(),
    })
}

fn write_parsed(db: &Db, p: &Parsed) -> Result<()> {
    let f = &p.file;
    let existing = match p.plan {
        Plan::Append(_) => db.session(&f.session_id)?.filter(|s| s.source == Source::Transcript),
        _ => None,
    };
    let mut s = existing.unwrap_or_else(|| SessionSummary::transcript(&f.session_id, &f.project_key, &f.path));
    if !matches!(p.plan, Plan::Append(_)) {
        db.delete_messages(&f.session_id)?;
        db.delete_usage(&f.session_id, Some(""))?;
    }
    p.delta.apply(&mut s);
    s.file_path = Some(f.path.clone());
    s.favorite = f.favorite;
    s.source = Source::Transcript;
    s.transcript_missing = false;
    s.updated_at = f.mtime_ms;
    s.file_size = Some(f.size);
    s.subagent_count = p.subagent_count;
    let project_id = db.upsert_project(&f.project_key, s.cwd.as_deref())?;
    db.upsert_session(&s, project_id)?;
    db.insert_messages(&f.session_id, &p.messages)?;
    db.insert_usage(&f.session_id, "", &p.usage)?;
    db.refresh_session_usage(&f.session_id)?;
    db.set_index_state(&f.path, &index_state(p))?;
    Ok(())
}

/// A subagent transcript only adds its usage to the session it belongs to. That row may not
/// exist yet; its totals are refreshed once it is written.
fn write_subagent(db: &Db, p: &Parsed) -> Result<()> {
    let f = &p.file;
    if !matches!(p.plan, Plan::Append(_)) {
        db.delete_usage(&f.session_id, Some(&f.agent))?;
    }
    db.insert_usage(&f.session_id, &f.agent, &p.usage)?;
    db.set_index_state(&f.path, &index_state(p))?;
    Ok(())
}

fn index_state(p: &Parsed) -> IndexState {
    IndexState {
        session_id: p.file.session_id.clone(),
        agent: p.file.agent.clone(),
        size: p.file.size,
        mtime_ns: p.file.mtime_ns,
        inode: Some(p.file.inode),
        indexed_offset: p.indexed_offset,
    }
}

fn emit_session(db: &Db, id: &str, emit: &mut dyn FnMut(ScanEvent)) -> Result<()> {
    if let Some(s) = db.session(id)? {
        emit(ScanEvent::Session(s));
    }
    Ok(())
}

/// Files that were indexed before and are gone now, split by kind.
struct Gone<'a> {
    sessions: Vec<(&'a PathBuf, &'a IndexState)>,
    subagents: Vec<(&'a PathBuf, &'a IndexState)>,
}

impl<'a> Gone<'a> {
    fn find(states: &'a HashMap<PathBuf, IndexState>, parsed_now: &HashSet<PathBuf>) -> Gone<'a> {
        let (subagents, sessions) = states
            .iter()
            .filter(|(path, _)| !parsed_now.contains(*path))
            .partition(|(_, state)| !state.agent.is_empty());
        Gone { sessions, subagents }
    }
}

/// Session transcripts that are gone: keep the session row (and what it cost), flag it, drop
/// the messages. Returns the session ids that were newly marked.
fn mark_missing(db: &Db, gone: &[(&PathBuf, &IndexState)]) -> Result<Vec<String>> {
    let mut missing = Vec::new();
    for (path, state) in gone {
        db.mark_transcript_missing(&state.session_id)?;
        db.delete_messages(&state.session_id)?;
        db.delete_index_state(path)?;
        missing.push(state.session_id.clone());
    }
    Ok(missing)
}

/// Subagent transcripts that are gone. While their session's own transcript is still there the
/// subagent was deleted on its own and its usage goes too; when the whole session was cleaned up
/// its usage stays, like the session's own. Returns the sessions whose totals changed.
fn forget_subagents(db: &Db, gone: &[(&PathBuf, &IndexState)], live_ids: &HashSet<String>) -> Result<BTreeSet<String>> {
    let mut touched = BTreeSet::new();
    for (path, state) in gone {
        if live_ids.contains(&state.session_id) {
            db.delete_usage(&state.session_id, Some(&state.agent))?;
            touched.insert(state.session_id.clone());
        }
        db.delete_index_state(path)?;
    }
    Ok(touched)
}

#[derive(Debug)]
pub struct RootsChanged;
impl std::fmt::Display for RootsChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "RootsChanged: index roots differ; use --db or index --rebuild") }
}
impl std::error::Error for RootsChanged {}
