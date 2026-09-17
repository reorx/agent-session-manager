//! Incremental indexer: stat every transcript, parse only what changed (in parallel), write in
//! one transaction, then fill gaps from `history.jsonl`. See `kb/docs/data-layer-design.md` §4.

use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use rayon::prelude::*;

use super::db::{Db, IndexState};
use crate::claude::discover::{TranscriptFile, discover_transcripts, subagent_count};
use crate::claude::history::{history_sessions, load_history};
use crate::claude::parse::{SessionDelta, TranscriptParser, map_file};
use crate::claude::paths::ClaudeDir;
use crate::model::{MessageRow, SessionSummary, Source};

pub struct Indexer {
    claude: ClaudeDir,
}

/// Scan progress, reported after every file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub phase: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub files_seen: usize,
    /// New files parsed from the start.
    pub files_parsed: usize,
    pub files_appended: usize,
    pub files_rebuilt: usize,
    pub files_unchanged: usize,
    pub files_missing: usize,
    pub bytes_parsed: u64,
    /// Sessions written from history.jsonl (new or refreshed).
    pub history_sessions: usize,
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
    /// Absolute offset up to which the file has been parsed.
    indexed_offset: u64,
    bytes_parsed: u64,
    subagent_count: u32,
}

impl Indexer {
    pub fn new(claude: ClaudeDir) -> Self {
        Indexer { claude }
    }

    /// Bring the index up to date with the files on disk.
    pub fn scan(&self, db: &mut Db, progress: &mut dyn FnMut(&Progress)) -> Result<ScanReport> {
        let started = Instant::now();
        let mut report = ScanReport::default();
        let files = discover_transcripts(&self.claude)?;
        report.files_seen = files.len();
        let states = db.all_index_states()?;
        let planned: Vec<(TranscriptFile, Plan)> =
            files.into_iter().map(|f| { let plan = plan_for(&f, states.get(&f.path)); (f, plan) }).collect();
        report.files_unchanged = planned.iter().filter(|(_, p)| *p == Plan::Unchanged).count();
        progress(&Progress { done: report.files_unchanged, total: report.files_seen, phase: "scanning" });

        let to_parse: Vec<(TranscriptFile, Plan)> = planned.into_iter().filter(|(_, p)| *p != Plan::Unchanged).collect();
        let t = Instant::now();
        let parsed: Vec<Parsed> = to_parse.into_par_iter().filter_map(|(f, plan)| parse_file(f, plan).ok()).collect();
        report.elapsed_parse = t.elapsed();

        let mut done = report.files_unchanged;
        let t = Instant::now();
        db.transaction(|db| {
            for p in &parsed {
                write_parsed(db, p)?;
                match p.plan {
                    Plan::Full => report.files_parsed += 1,
                    Plan::Append(_) => report.files_appended += 1,
                    Plan::Rebuild => report.files_rebuilt += 1,
                    Plan::Unchanged => {}
                }
                report.bytes_parsed += p.bytes_parsed;
                done += 1;
                progress(&Progress { done, total: report.files_seen, phase: "indexing" });
            }
            let on_disk: std::collections::HashSet<&PathBuf> = parsed.iter().map(|p| &p.file.path).collect();
            report.files_missing = mark_missing(db, &states, &on_disk, report.files_unchanged)?;
            report.elapsed_write = t.elapsed();
            let t = Instant::now();
            report.history_sessions = self.import_history(db)?;
            report.elapsed_history = t.elapsed();
            Ok(())
        })?;
        progress(&Progress { done: report.files_seen, total: report.files_seen, phase: "done" });
        report.elapsed = started.elapsed();
        Ok(report)
    }

    /// Drop the whole index and scan again.
    pub fn rebuild(&self, db: &mut Db, progress: &mut dyn FnMut(&Progress)) -> Result<ScanReport> {
        db.clear_all()?;
        self.scan(db, progress)
    }

    /// Sessions that exist only in history.jsonl (transcript gone or never indexed).
    fn import_history(&self, db: &Db) -> Result<usize> {
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
                db.upsert_session(&s, project_id)?;
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
    let mut parser = TranscriptParser::new(&file.session_id);
    let feed = parser.feed(&bytes[from as usize..], from);
    let subagents = subagent_count(&file.path);
    Ok(Parsed {
        plan,
        delta: parser.into_delta(),
        messages: feed.messages,
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
    }
    p.delta.apply(&mut s);
    s.file_path = Some(f.path.clone());
    s.source = Source::Transcript;
    s.transcript_missing = false;
    s.updated_at = f.mtime_ms;
    s.file_size = Some(f.size);
    s.subagent_count = p.subagent_count;
    let project_id = db.upsert_project(&f.project_key, s.cwd.as_deref())?;
    db.upsert_session(&s, project_id)?;
    db.insert_messages(&f.session_id, &p.messages)?;
    db.set_index_state(
        &f.path,
        &IndexState {
            session_id: f.session_id.clone(),
            size: f.size,
            mtime_ns: f.mtime_ns,
            inode: Some(f.inode),
            indexed_offset: p.indexed_offset,
        },
    )?;
    Ok(())
}

/// Files that were indexed before but are gone now: keep the session row, flag it, drop the
/// messages. Returns how many were newly marked.
fn mark_missing(
    db: &Db,
    states: &HashMap<PathBuf, IndexState>,
    parsed_now: &std::collections::HashSet<&PathBuf>,
    _unchanged: usize,
) -> Result<usize> {
    let mut missing = 0;
    for (path, state) in states {
        if parsed_now.contains(path) || path.is_file() {
            continue;
        }
        db.mark_transcript_missing(&state.session_id)?;
        db.delete_messages(&state.session_id)?;
        db.delete_index_state(path)?;
        missing += 1;
    }
    Ok(missing)
}
