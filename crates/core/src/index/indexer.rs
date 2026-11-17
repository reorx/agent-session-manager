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
    /// Parsed but not written: another connection indexed the file after this scan planned it.
    pub files_stale: usize,
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
    cache_events: Vec<crate::model::CacheEvent>,
    file: TranscriptFile,
    plan: Plan,
    delta: SessionDelta,
    messages: Vec<MessageRow>,
    usage: Vec<UsageRow>,
    /// Absolute offset up to which the file has been parsed.
    indexed_offset: u64,
    bytes_parsed: u64,
    subagent_count: u32,
    /// The file's index state this scan planned from (`None`: not indexed yet).
    planned_from: Option<IndexState>,
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
        check_roots(db, &claude_root, &favorites_root)?;
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

        let to_parse: Vec<(TranscriptFile, Plan)> = planned.into_iter().filter(|(_, p)| *p != Plan::Unchanged).collect();
        // An appending parser has to know the user rows already indexed (see TranscriptParser).
        // They belong to the plan, read right after the index states: the write transaction
        // drops a file whose state moved since (`stale_files`), and with it the uuids read here.
        // Re-planning inside the transaction instead would mean re-reading both.
        let mut seen_user: HashMap<PathBuf, HashSet<String>> = HashMap::new();
        for (f, _) in to_parse.iter().filter(|(f, p)| f.agent.is_empty() && matches!(p, Plan::Append(_))) {
            seen_user.insert(f.path.clone(), db.user_uuids(&f.session_id)?);
        }
        emit(ScanEvent::Progress(Progress { done: report.files_unchanged, total: report.files_seen, phase: "scanning" }));
        let t = Instant::now();
        let parsed: Vec<Parsed> = to_parse
            .into_par_iter()
            .map(|(f, plan)| {
                let seen = seen_user.get(&f.path).cloned().unwrap_or_default();
                let planned_from = states.get(&f.path).cloned();
                Ok(Parsed { planned_from, ..parse_file(f, plan, seen)? })
            })
            .collect::<Result<Vec<_>>>()?;
        report.elapsed_parse = t.elapsed();
        let (sessions, subagents): (Vec<&Parsed>, Vec<&Parsed>) = parsed.iter().partition(|p| p.file.agent.is_empty());

        let mut done = report.files_unchanged;
        let t = Instant::now();
        db.transaction(|db| {
            // another process may have built the index from other roots while this one waited
            check_roots(db, &claude_root, &favorites_root)?;
            db.set_meta("claude_root", &claude_root)?;
            db.set_meta("favorites_root", &favorites_root)?;
            for (path, state) in &gone.sessions {
                if live_ids.contains(&state.session_id) { db.delete_index_state(path)?; report.files_superseded += 1; }
            }
            // Subagent usage first, so every session row written below already counts it.
            // Sessions whose subagents changed but whose own transcript did not are sent at the end.
            let mut touched: BTreeSet<String> = forget_subagents(db, &gone.subagents, &live_ids)?;
            let stale = stale_files(db, &parsed)?;
            for p in subagents.iter().filter(|p| !stale.contains(&p.file.path)) {
                write_subagent(db, p)?;
                touched.insert(p.file.session_id.clone());
                report.subagents_parsed += 1;
                report.bytes_parsed += p.bytes_parsed;
            }
            for p in &sessions {
                if stale.contains(&p.file.path) {
                    report.files_stale += 1;
                    done += 1;
                    continue;
                }
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

    /// Whether a scan has gone through on this index since it was built or emptied: the scan
    /// writes `claude_root` in its transaction, and neither a new schema nor `clear_all` has it.
    /// Until then the index has nothing to answer from.
    pub fn has_scanned(db: &Db) -> Result<bool> {
        Ok(db.meta("claude_root")?.is_some())
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

/// `seen_user`: uuids of the user rows indexed from this file, used only when appending.
fn parse_file(file: TranscriptFile, plan: Plan, seen_user: HashSet<String>) -> Result<Parsed> {
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
    let mut parser = match (is_session, plan) {
        (false, _) => TranscriptParser::usage_only(),
        (true, Plan::Append(_)) => TranscriptParser::new(&file.session_id).seen_user_uuids(seen_user),
        (true, _) => TranscriptParser::new(&file.session_id),
    };
    let feed = parser.feed(&bytes[from as usize..], from);
    let subagents = if is_session { subagent_count(&file.path) } else { 0 };
    Ok(Parsed {
        plan,
        delta: parser.into_delta(),
        cache_events: feed.cache_events,
        messages: feed.messages,
        usage: feed.usage,
        indexed_offset: from + feed.consumed,
        bytes_parsed: len - from,
        subagent_count: subagents,
        file: file.clone(),
        planned_from: None,
    })
}

/// Files another connection indexed after this scan read the index states (the app and `asm`,
/// or two `asm`, refreshing at once): their state is not the one this scan planned from, and
/// writing them would add the same lines twice. That writer left them up to date, or the next
/// scan carries on from where it stopped. Read inside the write transaction, so it is current.
fn stale_files(db: &Db, parsed: &[Parsed]) -> Result<HashSet<PathBuf>> {
    let mut stale = HashSet::new();
    for p in parsed {
        if db.index_state(&p.file.path)? != p.planned_from {
            stale.insert(p.file.path.clone());
        }
    }
    Ok(stale)
}

fn write_parsed(db: &Db, p: &Parsed) -> Result<()> {
    let f = &p.file;
    let existing = match p.plan {
        Plan::Append(_) => db.session(&f.session_id)?.filter(|s| s.source == Source::Transcript),
        _ => None,
    };
    let mut s = existing.unwrap_or_else(|| SessionSummary::transcript(&f.session_id, &f.project_key, &f.path));
    if !matches!(p.plan, Plan::Append(_)) {
        db.delete_cache_facts(&f.session_id)?;
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
    db.insert_cache_events(&f.session_id, &p.cache_events)?;
    db.refresh_cache_breaks(&f.session_id)?;
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

/// The index must hold the sessions of one Claude directory and one favorites directory: the
/// ones it was built from.
fn check_roots(db: &Db, claude_root: &str, favorites_root: &str) -> Result<()> {
    for (key, value) in [("claude_root", claude_root), ("favorites_root", favorites_root)] {
        if db.meta(key)?.is_some_and(|previous| previous != value) { return Err(RootsChanged.into()); }
    }
    Ok(())
}

#[derive(Debug)]
pub struct RootsChanged;
impl std::fmt::Display for RootsChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "RootsChanged: index roots differ; use --db or index --rebuild") }
}
impl std::error::Error for RootsChanged {}
