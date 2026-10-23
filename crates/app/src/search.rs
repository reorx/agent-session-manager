//! Search bridge: a second background thread with its own query-only connection to the index,
//! so a search never queues behind a scan or a `Messages` request of the store thread.
//!
//! The UI sends the query together with the sessions that pass the current filters (newest
//! first); text mode asks FTS5, regex mode scans those sessions' transcripts. Every request
//! supersedes the previous one: `latest` is bumped on send, a running regex scan sees it and
//! stops, and events of an old `seq` are dropped by the receiver.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use agent_session_core::claude::discover::TranscriptFile;
use agent_session_core::index::Db;
use agent_session_core::search::fts::{self, FtsQuery, FtsRoles};
use agent_session_core::search::regex::{FileMatches, Match, RegexOptions, RoleSet, Snippet, ToolLineKind, scan_sessions};
use agent_session_core::search::{Highlighted, Matcher};
use anyhow::Result;
use async_channel::{Receiver, Sender};
use gpui_kit::SharedString;
use serde::{Deserialize, Serialize};

use crate::model::ScopeSet;

/// Sessions a search shows at most (the newest ones that match).
pub const MAX_RESULT_SESSIONS: usize = 200;
/// Characters kept ahead of the first highlight of a preview snippet, and the snippet's length.
/// The Preview cell cuts on the right and may be as narrow as 30 CJK characters, so the lead is short.
const SNIPPET_BEFORE: usize = 12;
const SNIPPET_CHARS: usize = 240;
/// Characters a regex scan keeps on each side of a match (`focus` trims the front afterwards).
const REGEX_CONTEXT: usize = 120;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum SearchMode {
    /// Words, case-insensitive, all of them must appear (FTS5 over user / assistant text).
    #[default]
    Text,
    /// Regex over the transcripts; the only mode that reaches tool input / output and thinking.
    Regex,
}

/// Label filters offered in regex mode: (label, hint). A piece is kept when its label contains
/// one of the picked ones, so `tool_use` covers every tool.
pub const LABELS: [(&str, &str); 8] = [
    ("thinking", "reasoning"),
    ("tool_use", "any tool call"),
    ("tool_use:Bash", ""),
    ("tool_use:Read", ""),
    ("tool_use:Edit", ""),
    ("tool_use:Write", ""),
    ("tool_use:Grep", ""),
    ("tool_result", "tool output"),
];

/// How to search; everything but the query itself, and what survives a restart.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchOptions {
    pub mode: SearchMode,
    /// Regex mode only; text mode never distinguishes case.
    pub case_sensitive: bool,
    pub scope: ScopeSet,
    /// Regex mode only.
    pub labels: Vec<String>,
}

impl SearchOptions {
    pub fn toggle_label(&mut self, label: &str) {
        match self.labels.iter().position(|l| l == label) {
            Some(ix) => drop(self.labels.remove(ix)),
            None => self.labels.push(label.to_string()),
        }
    }

    pub fn labels_label(&self) -> String {
        match self.labels.as_slice() {
            [] => "Any".into(),
            [one] => one.clone(),
            many => format!("{} labels", many.len()),
        }
    }

    fn fts_roles(&self) -> FtsRoles {
        FtsRoles { user: self.scope.user, assistant: self.scope.assistant }
    }

    /// A picked label brings its role along: `thinking` lives in assistant records, the tool
    /// labels in tool pieces, whatever the Scope dropdown says.
    fn regex_options(&self) -> RegexOptions {
        let mut roles = RoleSet { user: self.scope.user, assistant: self.scope.assistant, tool: self.scope.tools };
        roles.assistant |= self.labels.iter().any(|l| l == "thinking");
        roles.tool |= self.labels.iter().any(|l| l.starts_with("tool"));
        RegexOptions { roles, labels: self.labels.clone(), case_sensitive: self.case_sensitive, context: REGEX_CONTEXT, limit: usize::MAX }
    }

    /// What to highlight in full texts for `query`; `None` for a blank query.
    pub fn matcher(&self, query: &str) -> Option<Matcher> {
        match self.mode {
            SearchMode::Text => Matcher::words(query),
            SearchMode::Regex => Some(Matcher::regex(query.trim(), self.case_sensitive)).filter(|_| !query.trim().is_empty()),
        }
    }
}

/// One session a search may look into.
#[derive(Clone, Debug)]
pub struct SearchTarget {
    pub session_id: String,
    pub project_key: String,
    /// `None` for history-only sessions: nothing to scan in regex mode.
    pub path: Option<PathBuf>,
    pub updated_at: i64,
}

/// What the table shows for a session with matches.
#[derive(Debug, PartialEq)]
pub struct RowHit {
    pub count: u32,
    /// Where the first match is: `user`, `assistant`, `thinking`, `tool_use:Bash`, `tool_result`.
    pub label: SharedString,
    pub snippet: SharedString,
    pub ranges: Vec<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchSummary {
    /// Sessions handed to the table.
    pub sessions: usize,
    /// Sessions that match, when the search knows (text mode counts them all, a regex scan stops early).
    pub total_sessions: Option<usize>,
    pub matches: usize,
    /// More sessions match than `MAX_RESULT_SESSIONS`.
    pub truncated: bool,
    pub elapsed: Duration,
}

#[derive(Debug)]
pub enum SearchEvent {
    Hits { seq: u64, hits: Vec<(SharedString, Arc<RowHit>)> },
    Done { seq: u64, summary: SearchSummary },
    Failed { seq: u64, message: String },
}

struct SearchRequest {
    seq: u64,
    query: String,
    options: SearchOptions,
    targets: Vec<SearchTarget>,
}

pub struct Searcher {
    requests: mpsc::Sender<SearchRequest>,
    latest: Arc<AtomicU64>,
}

impl Searcher {
    pub fn spawn(db_path: PathBuf) -> (Searcher, Receiver<SearchEvent>) {
        let (requests, request_rx) = mpsc::channel();
        let (event_tx, events) = async_channel::unbounded();
        let latest = Arc::new(AtomicU64::new(0));
        let worker = Worker { db_path, db: None, latest: latest.clone(), events: event_tx };
        std::thread::Builder::new()
            .name("asm-search".into())
            .spawn(move || run(worker, request_rx))
            .expect("failed to spawn the search thread");
        (Searcher { requests, latest }, events)
    }

    /// Start a search, superseding whatever is running. Returns the `seq` its events carry.
    pub fn search(&self, query: String, options: SearchOptions, targets: Vec<SearchTarget>) -> u64 {
        let seq = self.cancel();
        self.requests.send(SearchRequest { seq, query, options, targets }).ok();
        seq
    }

    /// Stop the running search; events of older searches are stale from now on.
    pub fn cancel(&self) -> u64 {
        self.latest.fetch_add(1, Ordering::SeqCst) + 1
    }
}

/// Thread body: a failed search becomes a `Failed` event and drops the connection, so the next
/// search opens a fresh one.
fn run(mut worker: Worker, requests: mpsc::Receiver<SearchRequest>) {
    for request in requests {
        if worker.is_stale(request.seq) {
            continue;
        }
        let started = Instant::now();
        let event = match worker.search(&request) {
            Ok(Some(found)) => SearchEvent::Done { seq: request.seq, summary: found.summary(started.elapsed()) },
            Ok(None) => continue,
            Err(e) => {
                worker.db = None;
                SearchEvent::Failed { seq: request.seq, message: format!("{e:#}") }
            }
        };
        worker.events.send_blocking(event).ok();
    }
}

struct Found {
    sessions: usize,
    total_sessions: Option<usize>,
    matches: usize,
    truncated: bool,
}

impl Found {
    fn summary(self, elapsed: Duration) -> SearchSummary {
        SearchSummary { sessions: self.sessions, total_sessions: self.total_sessions, matches: self.matches, truncated: self.truncated, elapsed }
    }
}

struct Worker {
    db_path: PathBuf,
    /// Opened by the first text search: by then the store thread has created the index.
    db: Option<Db>,
    latest: Arc<AtomicU64>,
    events: Sender<SearchEvent>,
}

impl Worker {
    fn is_stale(&self, seq: u64) -> bool {
        self.latest.load(Ordering::SeqCst) != seq
    }

    /// `None` when the search was superseded half-way.
    fn search(&mut self, request: &SearchRequest) -> Result<Option<Found>> {
        match request.options.mode {
            SearchMode::Text => self.text_search(request).map(Some),
            SearchMode::Regex => Ok(self.regex_scan(request)),
        }
    }

    fn text_search(&mut self, request: &SearchRequest) -> Result<Found> {
        if self.db.is_none() {
            self.db = Some(Db::open_read_only(&self.db_path)?);
        }
        let db = self.db.as_ref().expect("opened above");
        let query = FtsQuery {
            text: request.query.clone(),
            roles: request.options.fts_roles(),
            project: None,
            since_ms: None,
            sessions: Some(request.targets.iter().map(|t| t.session_id.clone()).collect()),
            limit: MAX_RESULT_SESSIONS,
        };
        let outcome = fts::search(db, &query)?;
        let hits: Vec<(SharedString, Arc<RowHit>)> = outcome
            .hits
            .iter()
            .map(|hit| (hit.session_id.clone().into(), Arc::new(row_hit(hit.count, hit.first_role.as_str(), &hit.first))))
            .collect();
        let found = Found {
            sessions: hits.len(),
            total_sessions: Some(outcome.total_sessions),
            matches: outcome.total_matches,
            truncated: outcome.total_sessions > hits.len(),
        };
        self.events.send_blocking(SearchEvent::Hits { seq: request.seq, hits })?;
        Ok(found)
    }

    fn regex_scan(&self, request: &SearchRequest) -> Option<Found> {
        let seq = request.seq;
        let files: Vec<TranscriptFile> = request.targets.iter().filter_map(transcript_of).collect();
        let events = self.events.clone();
        // the scan checks from rayon's threads; `self` holds the connection and is not `Sync`
        let latest = self.latest.clone();
        let scan = scan_sessions(
            &request.query,
            &files,
            &request.options.regex_options(),
            MAX_RESULT_SESSIONS,
            &|| latest.load(Ordering::SeqCst) != seq,
            &mut |batch| {
                let hits = batch.iter().map(|file| (file.session_id.clone().into(), Arc::new(file_hit(file)))).collect();
                events.send_blocking(SearchEvent::Hits { seq, hits }).ok();
            },
        );
        (!scan.cancelled).then_some(Found { sessions: scan.sessions, total_sessions: None, matches: scan.matches, truncated: scan.truncated })
    }
}

/// Only the path is read by the scan; the file's stat fields are the indexer's business.
fn transcript_of(target: &SearchTarget) -> Option<TranscriptFile> {
    Some(TranscriptFile {
        project_key: target.project_key.clone(),
        session_id: target.session_id.clone(),
        agent: String::new(),
        path: target.path.clone()?,
        size: 0,
        mtime_ms: target.updated_at,
        mtime_ns: 0,
        inode: 0,
    })
}

fn row_hit(count: u32, label: &str, snippet: &Highlighted) -> RowHit {
    let shown = focus(snippet, SNIPPET_BEFORE, SNIPPET_CHARS);
    RowHit { count, label: label.to_string().into(), snippet: shown.text.into(), ranges: shown.ranges }
}

fn file_hit(file: &FileMatches) -> RowHit {
    let count = file.match_count.min(u32::MAX as usize) as u32;
    match file.matches.first() {
        Some(first) => row_hit(count, match_label(first), &match_line(first)),
        None => row_hit(count, "", &Highlighted::default()),
    }
}

fn match_label(m: &Match) -> &str {
    if m.detail.is_empty() { m.role.as_str() } else { &m.detail }
}

/// A tool call is several lines (`# description`, `$ command`, `key: value`); the preview shows
/// the first one that carries a highlight.
fn match_line(m: &Match) -> Highlighted {
    let lines = match &m.snippet {
        Snippet::Text(text) => return text.clone(),
        Snippet::ToolUse(lines) => lines,
    };
    let Some(line) = lines.iter().find(|l| !l.text.ranges.is_empty()).or(lines.first()) else { return Highlighted::default() };
    let prefix = match &line.kind {
        ToolLineKind::Description => "# ".to_string(),
        ToolLineKind::Command => "$ ".to_string(),
        ToolLineKind::Field(key) => format!("{key}: "),
    };
    Highlighted {
        text: format!("{prefix}{}", line.text.text),
        ranges: line.text.ranges.iter().map(|r| r.start + prefix.len()..r.end + prefix.len()).collect(),
    }
}

/// One table line out of a snippet: line breaks become `⏎`, at most `before` characters stay
/// ahead of the first highlight (the cell cuts on the right) and `max` characters overall.
fn focus(h: &Highlighted, before: usize, max: usize) -> Highlighted {
    let first = h.ranges.first().map_or(0, |r| r.start);
    let start = h.text[..first].char_indices().rev().nth(before.saturating_sub(1)).map_or(0, |(i, _)| i);
    let end = h.text[start..].char_indices().nth(max).map_or(h.text.len(), |(i, _)| start + i);
    let mut out = Highlighted::default();
    if start > 0 {
        out.text.push('…');
    }
    let mut pos = start;
    for r in &h.ranges {
        let (from, to) = (r.start.max(pos), r.end.min(end));
        if from >= to {
            continue;
        }
        out.text.push_str(&h.text[pos..from].replace('\n', "⏎"));
        let mark = out.text.len();
        out.text.push_str(&h.text[from..to].replace('\n', "⏎"));
        out.ranges.push(mark..out.text.len());
        pos = to;
    }
    out.text.push_str(&h.text[pos..end].replace('\n', "⏎"));
    if end < h.text.len() {
        out.text.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    // `gpui_kit::*` exports gpui's own `test` attribute; name the standard one explicitly.
    use core::prelude::v1::test;

    use agent_session_core::claude::paths::ClaudeDir;
    use agent_session_core::index::{Indexer, ListFilter, Order};

    use super::*;

    const S1: &str = "11111111-aaaa-4bbb-8ccc-000000000001";
    const S3: &str = "33333333-aaaa-4bbb-8ccc-000000000003";

    struct Fixture {
        _dir: tempfile::TempDir,
        searcher: Searcher,
        events: Receiver<SearchEvent>,
        targets: Vec<SearchTarget>,
    }

    /// The core fixture tree, indexed into a throw-away database; every session is a target.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("index.sqlite");
        let claude = ClaudeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../core/tests/fixtures/claude"));
        let mut db = Db::open(&db_path).unwrap();
        Indexer::new(claude).scan(&mut db, &mut |_| {}).unwrap();
        let sessions = db.list_sessions(&ListFilter { order: Order::UpdatedDesc, ..Default::default() }).unwrap();
        let targets = sessions
            .iter()
            .map(|s| SearchTarget {
                session_id: s.id.clone(),
                project_key: s.project_key.clone(),
                path: s.file_path.clone().filter(|_| !s.history_only()),
                updated_at: s.updated_at,
            })
            .collect();
        let (searcher, events) = Searcher::spawn(db_path);
        Fixture { _dir: dir, searcher, events, targets }
    }

    fn regex(scope: ScopeSet) -> SearchOptions {
        SearchOptions { mode: SearchMode::Regex, scope, ..Default::default() }
    }

    fn everything() -> ScopeSet {
        ScopeSet { user: true, assistant: true, tools: true }
    }

    /// Run one search to its `Done`: the hits in arrival order and the summary.
    fn run(f: &Fixture, query: &str, options: SearchOptions, targets: Vec<SearchTarget>) -> (Vec<(SharedString, Arc<RowHit>)>, SearchSummary) {
        let seq = f.searcher.search(query.into(), options, targets);
        let mut all = Vec::new();
        loop {
            match f.events.recv_blocking().expect("search thread ended early") {
                SearchEvent::Hits { seq: s, hits } if s == seq => all.extend(hits),
                SearchEvent::Done { seq: s, summary } if s == seq => return (all, summary),
                SearchEvent::Failed { message, .. } => panic!("search failed: {message}"),
                _ => {}
            }
        }
    }

    fn marked(hit: &RowHit) -> Vec<&str> {
        hit.ranges.iter().map(|r| &hit.snippet[r.clone()]).collect()
    }

    #[test]
    fn a_text_search_returns_the_sessions_with_hits_a_count_and_a_highlighted_snippet() {
        let f = fixture();
        let (hits, summary) = run(&f, "parser", SearchOptions::default(), f.targets.clone());
        assert_eq!(hits.len(), 1);
        let (id, hit) = &hits[0];
        assert_eq!(id.as_ref(), S1);
        assert_eq!(hit.count, 3);
        assert_eq!(hit.label, "assistant", "the first hit in conversation order is a reply");
        assert_eq!(marked(hit), ["parser"]);
        assert_eq!((summary.sessions, summary.total_sessions, summary.matches, summary.truncated), (1, Some(1), 3, false));
    }

    #[test]
    fn two_character_cjk_queries_find_their_sessions() {
        let f = fixture();
        let (hits, _) = run(&f, "谢谢", SearchOptions::default(), f.targets.clone());
        assert_eq!(hits.len(), 1);
        assert_eq!(marked(&hits[0].1), ["谢谢"]);
        let (three, _) = run(&f, "解析器", SearchOptions::default(), f.targets.clone());
        assert_eq!(three[0].0.as_ref(), S3);
    }

    #[test]
    fn the_scope_limits_a_text_search_to_prompts_or_replies() {
        let f = fixture();
        let user_only = SearchOptions { scope: ScopeSet { user: true, assistant: false, tools: false }, ..Default::default() };
        let (hits, _) = run(&f, "parser", user_only, f.targets.clone());
        assert_eq!((hits[0].1.count, hits[0].1.label.as_ref()), (1, "user"));
        let tools_only = SearchOptions { scope: ScopeSet { user: false, assistant: false, tools: true }, ..Default::default() };
        assert!(run(&f, "parser", tools_only, f.targets.clone()).0.is_empty(), "tool input / output is not in the index");
    }

    #[test]
    fn only_the_given_targets_are_searched() {
        let f = fixture();
        let others: Vec<SearchTarget> = f.targets.iter().filter(|t| t.session_id != S1).cloned().collect();
        let (hits, summary) = run(&f, "parser", SearchOptions::default(), others.clone());
        assert!(hits.is_empty() && summary.total_sessions == Some(0), "text mode");
        let (hits, summary) = run(&f, "parser", regex(everything()), others);
        assert!(hits.is_empty() && summary.sessions == 0, "regex mode");
    }

    #[test]
    fn a_regex_search_scans_the_transcripts_and_agrees_with_the_index_on_plain_text() {
        let f = fixture();
        let (hits, summary) = run(&f, r"pars\w+", regex(ScopeSet::default()), f.targets.clone());
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].0.as_ref(), hits[0].1.count), (S1, 3));
        assert_eq!(marked(&hits[0].1), ["parser"]);
        assert_eq!((summary.sessions, summary.total_sessions, summary.truncated), (1, None, false));
    }

    #[test]
    fn the_tools_scope_and_the_labels_reach_tool_calls() {
        let f = fixture();
        let without = run(&f, "cargo", regex(ScopeSet::default()), f.targets.clone()).0;
        let with = run(&f, "cargo", regex(everything()), f.targets.clone()).0;
        let total = |hits: &[(SharedString, Arc<RowHit>)]| hits.iter().map(|(_, h)| h.count).sum::<u32>();
        assert!(total(&with) > total(&without), "tool input / output adds matches: {} vs {}", total(&with), total(&without));

        let bash = SearchOptions { labels: vec!["tool_use:Bash".into()], ..regex(ScopeSet::default()) };
        let (hits, _) = run(&f, "cargo", bash, f.targets.clone());
        assert!(!hits.is_empty(), "a tool label searches tool calls even with the Tools scope off");
        assert!(hits.iter().all(|(_, h)| h.label == "tool_use:Bash"), "labels: {:?}", hits.iter().map(|(_, h)| h.label.clone()).collect::<Vec<_>>());
        assert!(hits[0].1.snippet.starts_with("$ ") || hits[0].1.snippet.starts_with("# "), "snippet: {}", hits[0].1.snippet);
        assert!(marked(&hits[0].1).iter().all(|m| m.eq_ignore_ascii_case("cargo")));
    }

    #[test]
    fn case_matters_only_when_asked_for() {
        let f = fixture();
        let sensitive = SearchOptions { case_sensitive: true, ..regex(ScopeSet::default()) };
        assert!(run(&f, "PARSER", sensitive, f.targets.clone()).0.is_empty());
        assert_eq!(run(&f, "PARSER", regex(ScopeSet::default()), f.targets.clone()).0.len(), 1);
    }

    #[test]
    fn a_newer_search_supersedes_the_older_one() {
        let f = fixture();
        let old = f.searcher.search("parser".into(), SearchOptions::default(), f.targets.clone());
        let (hits, _) = run(&f, "解析器", SearchOptions::default(), f.targets.clone());
        assert_eq!(hits[0].0.as_ref(), S3);
        assert!(f.searcher.cancel() > old + 1, "every search and every cancel moves the sequence on");
    }

    #[test]
    fn a_missing_index_is_reported_and_the_next_search_still_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (searcher, events) = Searcher::spawn(dir.path().join("nope.sqlite"));
        let seq = searcher.search("x y z".into(), SearchOptions::default(), Vec::new());
        assert!(matches!(events.recv_blocking().unwrap(), SearchEvent::Failed { seq: s, .. } if s == seq));
        let seq = searcher.search("xyz".into(), regex(ScopeSet::default()), Vec::new());
        assert!(matches!(events.recv_blocking().unwrap(), SearchEvent::Done { seq: s, .. } if s == seq), "a regex scan needs no index");
    }

    /// Not a behavior test: prints how long text searches take on a real index, in the same
    /// build profile as the debug app. `ASM_BENCH_DB=<index copy> cargo test -p agent-session-manager
    /// real_index_timings -- --ignored --nocapture`; `ASM_BENCH_QUERIES` = comma-separated queries.
    #[test]
    #[ignore]
    fn real_index_timings() {
        let db_path = PathBuf::from(std::env::var("ASM_BENCH_DB").expect("ASM_BENCH_DB points to an index"));
        let queries = std::env::var("ASM_BENCH_QUERIES").unwrap_or_else(|_| "session table,搜索,ab,罕见,的".into());
        let db = Db::open_read_only(&db_path).unwrap();
        let sessions = db.list_sessions(&ListFilter { order: Order::UpdatedDesc, ..Default::default() }).unwrap();
        let targets: Vec<SearchTarget> = sessions
            .iter()
            .map(|s| SearchTarget {
                session_id: s.id.clone(),
                project_key: s.project_key.clone(),
                path: s.file_path.clone().filter(|_| !s.history_only()),
                updated_at: s.updated_at,
            })
            .collect();
        let (searcher, events) = Searcher::spawn(db_path);
        let f = Fixture { _dir: tempfile::tempdir().unwrap(), searcher, events, targets };
        let timed = |query: &str, options: SearchOptions| {
            let runs: Vec<String> = (0..5)
                .map(|_| {
                    let wall = Instant::now();
                    let (_, summary) = run(&f, query, options.clone(), f.targets.clone());
                    format!("{:.1}/{:.1}", summary.elapsed.as_secs_f64() * 1000.0, wall.elapsed().as_secs_f64() * 1000.0)
                })
                .collect();
            let (hits, summary) = run(&f, query, options.clone(), f.targets.clone());
            println!("{query:>14}: {} sessions shown, {:?} total, {} matches; worker/wall ms: {}", hits.len(), summary.total_sessions, summary.matches, runs.join("  "));
        };
        for query in queries.split(',') {
            timed(query, SearchOptions::default());
        }
        if let Ok(pattern) = std::env::var("ASM_BENCH_REGEX") {
            timed(&pattern, regex(ScopeSet::default()));
            timed(&pattern, regex(everything()));
        }
    }

    #[test]
    fn preview_snippets_are_one_line_start_near_the_match_and_stay_short() {
        let text = format!("{}\nneedle{}", "a".repeat(100), "b".repeat(400));
        let h = Highlighted { ranges: vec![101..107], text };
        let shown = focus(&h, 10, 30);
        assert_eq!(shown.text, format!("…{}⏎needle{}…", "a".repeat(9), "b".repeat(14)));
        assert_eq!(&shown.text[shown.ranges[0].clone()], "needle");

        let short = Highlighted { text: "say 你好 twice 你好".into(), ranges: vec![4..10, 17..23] };
        let same = focus(&short, 40, 240);
        assert_eq!((same.text.as_str(), &same.ranges), ("say 你好 twice 你好", &short.ranges));
    }

    #[test]
    fn the_matcher_follows_the_mode() {
        let text = SearchOptions::default();
        assert_eq!(text.matcher("Plan parser").unwrap().ranges("the plan"), vec![4..8]);
        assert!(text.matcher("  ").is_none());
        let re = SearchOptions { mode: SearchMode::Regex, case_sensitive: true, ..Default::default() };
        assert!(re.matcher("Plan").unwrap().ranges("the plan").is_empty());
        assert!(re.matcher(" ").is_none());
    }

    #[test]
    fn labels_toggle_and_describe_themselves() {
        let mut o = SearchOptions::default();
        assert_eq!(o.labels_label(), "Any");
        o.toggle_label("thinking");
        assert_eq!(o.labels_label(), "thinking");
        o.toggle_label("tool_result");
        assert_eq!(o.labels_label(), "2 labels");
        o.toggle_label("thinking");
        assert_eq!(o.labels, ["tool_result"]);
        assert!(LABELS.iter().any(|(l, _)| *l == "tool_result"));
    }
}
