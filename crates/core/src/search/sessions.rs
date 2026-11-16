//! Session-level search (`asm search`): a session is a result when every TERM occurs somewhere in
//! it, in the text kinds asked for. The index answers when every kind asked for is indexed and the
//! TERMs are literal; tool kinds and regexes scan the transcripts (`scan.rs`). Both produce the
//! same `SessionHits`: the same items counted, the same snippets picked, the same turn numbers.

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::{Map, Value};

use super::Highlighted;
use super::query::{Query, TermMask, has_non_ascii_case, indexes};
use crate::claude::clean::clean_prompt;
use crate::cost::primary_model;
use crate::index::Db;
use crate::model::{SessionSummary, TextKind};
use crate::pricing::normalize;

pub use super::scan::{READ_BUDGET, ScanStats};

/// Characters a snippet keeps before the first hit of its item, and from its start.
pub const SNIPPET_BEFORE: usize = 60;
pub const SNIPPET_AFTER: usize = 140;

// ---------------------------------------------------------------------------------------------
// Text kinds

/// A text kind as `asm search --in` names it. The order is the one of the Text kinds table, which
/// is also the order snippets prefer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Prompt,
    Reply,
    Thinking,
    ToolInput,
    ToolOutput,
    Other,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Prompt, Kind::Reply, Kind::Thinking, Kind::ToolInput, Kind::ToolOutput, Kind::Other];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Prompt => "prompt",
            Kind::Reply => "reply",
            Kind::Thinking => "thinking",
            Kind::ToolInput => "tool-input",
            Kind::ToolOutput => "tool-output",
            Kind::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// The kind of the index rows that hold this text; `None` for tool kinds (never indexed).
    pub fn text_kind(self) -> Option<TextKind> {
        match self {
            Kind::Prompt => Some(TextKind::Prompt),
            Kind::Reply => Some(TextKind::Reply),
            Kind::Thinking => Some(TextKind::Thinking),
            Kind::Other => Some(TextKind::Other),
            Kind::ToolInput | Kind::ToolOutput => None,
        }
    }

    fn from_text_kind(k: TextKind) -> Kind {
        match k {
            TextKind::Prompt => Kind::Prompt,
            TextKind::Reply => Kind::Reply,
            TextKind::Thinking => Kind::Thinking,
            TextKind::Other => Kind::Other,
        }
    }

    fn is_tool(self) -> bool {
        matches!(self, Kind::ToolInput | Kind::ToolOutput)
    }
}

/// The kinds searched (`--in`), and the tools whose calls count (`--tool`, empty for all).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindSet {
    kinds: [bool; 6],
    /// Lowercase tool names.
    tools: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KindError {
    Unknown(String),
    Empty,
    /// `--tool` with an `--in` that names no tool kind.
    ToolWithoutToolKind,
}

impl std::fmt::Display for KindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KindError::Unknown(k) => write!(
                f,
                "unknown kind '{k}' in --in (kinds: prompt, reply, thinking, tool, tool-input, tool-output, other, all)"
            ),
            KindError::Empty => write!(f, "--in names no kind"),
            KindError::ToolWithoutToolKind => write!(f, "--tool needs a tool kind in --in (tool, tool-input or tool-output)"),
        }
    }
}

impl std::error::Error for KindError {}

impl Default for KindSet {
    /// `prompt,reply,thinking`.
    fn default() -> Self {
        KindSet::of(&[Kind::Prompt, Kind::Reply, Kind::Thinking])
    }
}

impl KindSet {
    pub fn of(kinds: &[Kind]) -> KindSet {
        let mut set = KindSet { kinds: [false; 6], tools: Vec::new() };
        for k in kinds {
            set.kinds[*k as usize] = true;
        }
        set
    }

    /// `--in` (comma-separated, `tool` and `all` allowed) and `--tool`: without `--in`, `--tool`
    /// searches the inputs and outputs of those tools only; with it, `--tool` narrows the tool kinds
    /// `--in` names and leaves the others alone.
    pub fn parse(in_: Option<&str>, tools: &[String]) -> Result<KindSet, KindError> {
        let mut set = match in_ {
            None if tools.is_empty() => KindSet::default(),
            None => KindSet::of(&[Kind::ToolInput, Kind::ToolOutput]),
            Some(list) => {
                let mut set = KindSet::of(&[]);
                for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                    match name {
                        "all" => set.kinds = [true; 6],
                        "tool" => {
                            set.kinds[Kind::ToolInput as usize] = true;
                            set.kinds[Kind::ToolOutput as usize] = true;
                        }
                        _ => set.kinds[Kind::parse(name).ok_or_else(|| KindError::Unknown(name.to_string()))? as usize] = true,
                    }
                }
                if set.kinds().is_empty() {
                    return Err(KindError::Empty);
                }
                if !tools.is_empty() && !set.contains(Kind::ToolInput) && !set.contains(Kind::ToolOutput) {
                    return Err(KindError::ToolWithoutToolKind);
                }
                set
            }
        };
        set.tools = tools.iter().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect();
        Ok(set)
    }

    pub fn contains(&self, kind: Kind) -> bool {
        self.kinds[kind as usize]
    }

    /// The kinds, in the order of the Text kinds table.
    pub fn kinds(&self) -> Vec<Kind> {
        Kind::ALL.into_iter().filter(|k| self.contains(*k)).collect()
    }

    /// The `--tool` names as given to `parse`, lowercased.
    pub fn tools(&self) -> &[String] {
        &self.tools
    }

    /// Every kind is in the index (`prompt`, `reply`, `thinking`, `other`).
    pub fn index_only(&self) -> bool {
        self.kinds().iter().all(|k| !k.is_tool())
    }

    /// Whether a call of `name` counts (`--tool`, exact and case-insensitive).
    pub fn tool_matches(&self, name: &str) -> bool {
        self.tools.is_empty() || {
            let name = name.to_lowercase();
            self.tools.contains(&name)
        }
    }
}

/// Which way answers a search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Index,
    Scan,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Index => "index",
            Engine::Scan => "scan",
        }
    }

    /// The index, unless a kind is not in it or the TERMs are regexes.
    pub fn for_query(query: &Query, kinds: &KindSet) -> Engine {
        if !query.regex && kinds.index_only() { Engine::Index } else { Engine::Scan }
    }
}

// ---------------------------------------------------------------------------------------------
// Which sessions are searched

/// `--project P`: a directory (with the projects inside it) or text in a project's path or
/// encoded directory name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectMatcher {
    /// The directory as written (absolute, `.` and `..` resolved without the disk), and with
    /// symlinks resolved (see `resolve_symlinks`) when that differs.
    Dir { written: PathBuf, resolved: Option<PathBuf> },
    /// Lowercase.
    Text(String),
}

impl ProjectMatcher {
    /// A directory when `arg` is `.` or `..`, contains `/` or starts with `~`; else text.
    /// Relative directories are taken from `cwd`, `~` is `home`.
    pub fn parse(arg: &str, cwd: &Path, home: &Path) -> ProjectMatcher {
        if !is_directory(arg) {
            return ProjectMatcher::Text(arg.to_lowercase());
        }
        let path = if arg == "~" {
            home.to_path_buf()
        } else if let Some(rest) = arg.strip_prefix("~/") {
            home.join(rest)
        } else {
            cwd.join(arg)
        };
        let written = normalize_path(&path);
        let resolved = resolve_symlinks(&written).filter(|r| *r != written);
        ProjectMatcher::Dir { written, resolved }
    }

    /// `parse` takes `arg` from the current directory: a relative directory. Asking for the
    /// current directory only then lets a search run from one that was deleted.
    pub fn needs_cwd(arg: &str) -> bool {
        is_directory(arg) && !(arg == "~" || arg.starts_with("~/")) && !Path::new(arg).is_absolute()
    }

    fn matches(&self, path: Option<&str>, key: &str, resolve: &mut impl FnMut(&str) -> Option<PathBuf>) -> bool {
        match self {
            ProjectMatcher::Text(text) => {
                path.is_some_and(|p| p.to_lowercase().contains(text.as_str())) || key.to_lowercase().contains(text.as_str())
            }
            ProjectMatcher::Dir { written, resolved } => {
                let needles = std::iter::once(written).chain(resolved.as_ref());
                match path {
                    Some(p) => {
                        let as_written = normalize_path(Path::new(p));
                        let real = resolve(p);
                        needles.into_iter().any(|n| as_written.starts_with(n) || real.as_ref().is_some_and(|r| r.starts_with(n)))
                    }
                    // the encoded name loses the path's structure: only the directory itself matches
                    None => needles.into_iter().any(|n| crate::claude::paths::encode_project_path(&n.to_string_lossy()) == key),
                }
            }
        }
    }
}

/// `--project P` names a directory, not text in project paths.
fn is_directory(arg: &str) -> bool {
    arg == "." || arg == ".." || arg.contains('/') || arg.starts_with('~')
}

/// The path with symlinks resolved as far as it exists: its longest existing ancestor is
/// resolved and the rest appended, so that a deleted `/tmp/x` is still `/private/tmp/x`.
fn resolve_symlinks(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|ancestor| {
        let real = ancestor.canonicalize().ok()?;
        let rest = path.strip_prefix(ancestor).expect("an ancestor is a prefix");
        Some(if rest.as_os_str().is_empty() { real } else { real.join(rest) })
    })
}

/// `.` and `..` resolved without looking at the disk.
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The sessions a search looks at.
#[derive(Clone, Debug, Default)]
pub struct SessionFilter {
    /// Any of them; empty for every project.
    pub projects: Vec<ProjectMatcher>,
    /// The time range, unix ms: activity (first prompt to last update) overlapping
    /// `[since_ms, until_ms)`.
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    /// Main model, `opus-5-5` or `claude-opus-5-5`.
    pub model: Option<String>,
    pub favorites: bool,
    /// The current session, left out.
    pub exclude: Option<String>,
}

impl SessionFilter {
    /// Whether the session's activity span overlaps the range; a session without a first prompt
    /// spans its last update alone.
    pub fn in_range(&self, s: &SessionSummary) -> bool {
        let start = s.started_at.unwrap_or(s.updated_at);
        self.since_ms.is_none_or(|since| s.updated_at >= since) && self.until_ms.is_none_or(|until| start < until)
    }
}

/// The sessions passing `filter`, most recently active first.
pub fn select_sessions(db: &Db, filter: &SessionFilter) -> Result<Vec<SessionSummary>> {
    let mut sessions = db.list_sessions(&crate::index::ListFilter {
        favorites_only: filter.favorites,
        since_ms: filter.since_ms,
        order: crate::index::Order::UpdatedDesc,
        ..Default::default()
    })?;
    let model = filter.model.as_deref().map(|m| {
        let id = if m.starts_with("claude-") { m.to_string() } else { format!("claude-{m}") };
        normalize(&id).to_string()
    });
    let mut real: HashMap<String, Option<PathBuf>> = HashMap::new();
    let mut resolve = |p: &str| real.entry(p.to_string()).or_insert_with(|| resolve_symlinks(Path::new(p))).clone();
    sessions.retain(|s| {
        filter.in_range(s)
            && filter.exclude.as_deref() != Some(s.id.as_str())
            && model.as_deref().is_none_or(|m| primary_model(&s.model_usage).is_some_and(|p| normalize(p) == m))
            && (filter.projects.is_empty()
                || filter.projects.iter().any(|p| p.matches(s.project_path.as_deref(), &s.project_key, &mut resolve)))
    });
    Ok(sessions)
}

// ---------------------------------------------------------------------------------------------
// Results

#[derive(Clone, Debug)]
pub struct SearchOptions {
    /// Snippets per session.
    pub snippets: usize,
    pub sort: SortBy,
    /// At most this many sessions in the outcome (the first ones in `sort` order); 0 for all.
    pub limit: usize,
    /// Force a way of searching (tests and benchmarks); the index refuses what it cannot answer.
    pub engine: Option<Engine>,
    /// The scan's byte prefilter (off only to check that it never changes a result).
    pub prefilter: bool,
    /// Bytes of transcripts the scan holds in memory at once (see `scan::Budget`).
    pub read_budget: u64,
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions { snippets: 3, sort: SortBy::Recent, limit: 0, engine: None, prefilter: true, read_budget: READ_BUDGET }
    }
}

#[derive(Clone, Debug)]
pub struct SearchOutcome {
    pub engine: Engine,
    /// Sessions found, before the limit.
    pub total: usize,
    /// In `sort` order, at most `limit`.
    pub sessions: Vec<SessionHits>,
    /// What the scan read and skipped; `None` for the index.
    pub scan: Option<ScanStats>,
}

/// One session found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHits {
    pub session_id: String,
    pub updated_at: i64,
    /// Items with at least one TERM, per kind searched (zeros included).
    pub counts: BTreeMap<Kind, u32>,
    /// Per TERM, the items that contain it.
    pub term_hits: Vec<u32>,
    /// The session's turns where this search numbered them (the index's prompt rows, or the
    /// prompts in the transcript the scan read), so no snippet's turn is above it.
    pub turns: u32,
    pub snippets: Vec<Hit>,
}

impl SessionHits {
    pub fn total(&self) -> u32 {
        self.counts.values().sum()
    }
}

/// One snippet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub kind: Kind,
    /// The tool's name for tool kinds (`None` when a result's call was not found).
    pub tool: Option<String>,
    /// `None` before the first prompt.
    pub turn: Option<u32>,
    pub ts: Option<i64>,
    /// Byte offset of the item's JSONL line in the transcript.
    pub offset: u64,
    /// The TERMs the item contains.
    pub terms: Vec<usize>,
    pub text: Highlighted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    /// Last activity, newest first.
    Recent,
    /// Most hits first, then newest.
    Hits,
}

impl SortBy {
    fn order(self, a: &SessionHits, b: &SessionHits) -> Ordering {
        let recent = b.updated_at.cmp(&a.updated_at).then_with(|| a.session_id.cmp(&b.session_id));
        match self {
            SortBy::Recent => recent,
            SortBy::Hits => b.total().cmp(&a.total()).then(recent),
        }
    }
}

/// How many of `total` sessions are shown.
fn shown(limit: usize, total: usize) -> usize {
    if limit == 0 { total } else { limit.min(total) }
}

/// A session's number of turns (its `prompt` rows in the index).
pub fn turn_count(db: &Db, session_id: &str) -> Result<u32> {
    let mut stmt = db.conn().prepare_cached("SELECT count(*) FROM messages WHERE session_id = ?1 AND kind = 'prompt'")?;
    Ok(stmt.query_row([session_id], |r| r.get(0))?)
}

/// The session's last prompt with text, cleaned of injected tags: the last `prompt` row, so never
/// a task notification or a compact summary (the parser's `SessionSummary::last_prompt` counts
/// those, the reference script's way). `None` without one, as for a history-only session.
pub fn last_prompt(db: &Db, session_id: &str) -> Result<Option<String>> {
    let mut stmt = db.conn().prepare_cached("SELECT text FROM messages WHERE session_id = ?1 AND kind = 'prompt' AND text <> '' ORDER BY byte_offset DESC")?;
    let mut rows = stmt.query([session_id])?;
    while let Some(row) = rows.next()? {
        let text = clean_prompt(&row.get::<_, String>(0)?);
        if !text.is_empty() {
            return Ok(Some(text));
        }
    }
    Ok(None)
}

/// Search `sessions` (from `select_sessions`).
pub fn search(db: &Db, sessions: &[SessionSummary], query: &Query, kinds: &KindSet, opts: &SearchOptions) -> Result<SearchOutcome> {
    let natural = Engine::for_query(query, kinds);
    let engine = opts.engine.unwrap_or(natural);
    if engine == Engine::Index && natural == Engine::Scan {
        bail!("the index cannot answer regexes or tool kinds");
    }
    Ok(match engine {
        Engine::Index => {
            let (sessions, total) = search_index(db, sessions, query, kinds, opts)?;
            SearchOutcome { engine, total, sessions, scan: None }
        }
        Engine::Scan => {
            // the snippets come with the reading, for every session found
            let (mut results, stats) = super::scan::search_scan(sessions, query, kinds, opts);
            results.sort_by(|a, b| opts.sort.order(a, b));
            let total = results.len();
            results.truncate(shown(opts.limit, total));
            SearchOutcome { engine, total, sessions: results, scan: Some(stats) }
        }
    })
}

// ---------------------------------------------------------------------------------------------
// Counting and picking, shared by both ways

/// Where an item sits and which TERMs it holds. `sub` tells apart the items of one line that
/// share a kind (the tool calls or results of one record).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ItemRef {
    pub kind: Kind,
    pub offset: u64,
    pub sub: u32,
    pub mask: TermMask,
}

impl ItemRef {
    /// Best first: more TERMs, then kind, then position.
    fn rank(&self) -> (Reverse<u32>, Kind, u64, u32) {
        (Reverse(self.mask.count_ones()), self.kind, self.offset, self.sub)
    }
}

/// Whether the session's items make it a result, and its counts: per kind searched, per TERM.
pub(crate) fn tally(items: &[ItemRef], query: &Query, kinds: &KindSet) -> Option<(BTreeMap<Kind, u32>, Vec<u32>)> {
    let all = query.all();
    let found = if query.same_message {
        items.iter().any(|i| i.mask == all)
    } else {
        items.iter().fold(0, |acc, i| acc | i.mask) == all
    };
    if !found {
        return None;
    }
    let mut counts: BTreeMap<Kind, u32> = kinds.kinds().into_iter().map(|k| (k, 0)).collect();
    let mut terms = vec![0; query.len()];
    for item in items {
        *counts.entry(item.kind).or_default() += 1;
        for t in indexes(item.mask) {
            terms[t] += 1;
        }
    }
    Some((counts, terms))
}

/// The items to show, best first: the first picks make sure every TERM is shown once, the rest
/// go by rank. Indexes into `items`.
pub(crate) fn pick(items: &[ItemRef], limit: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| items[i].rank());
    let mut picked: Vec<usize> = Vec::new();
    let mut covered: TermMask = 0;
    for &i in &order {
        if picked.len() >= limit {
            break;
        }
        if items[i].mask & !covered != 0 {
            picked.push(i);
            covered |= items[i].mask;
        }
    }
    for &i in &order {
        if picked.len() >= limit {
            break;
        }
        if !picked.contains(&i) {
            picked.push(i);
        }
    }
    picked.sort_by_key(|&i| items[i].rank());
    picked
}

/// A tool call's input as searched and shown: `# description`, `$ command`, then `key: value`
/// for the other fields (strings as they are, other values as JSON).
pub fn tool_input_text(input: &Map<String, Value>) -> String {
    use super::pyjson::stringify;
    let mut lines = Vec::new();
    if let Some(d) = input.get("description") {
        lines.push(format!("# {}", stringify(d)));
    }
    if let Some(c) = input.get("command") {
        lines.push(format!("$ {}", stringify(c)));
    }
    for (k, v) in input {
        if k != "description" && k != "command" {
            lines.push(format!("{k}: {}", stringify(v)));
        }
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------------------------
// The index

/// One index row with a TERM.
struct Row {
    id: i64,
    session: usize,
    kind: Kind,
    offset: u64,
    ts: i64,
    mask: TermMask,
}

impl Row {
    fn item(&self) -> ItemRef {
        ItemRef { kind: self.kind, offset: self.offset, sub: 0, mask: self.mask }
    }
}

/// The sessions shown, with their snippets and turns (looked up for those only), and how many
/// were found.
fn search_index(db: &Db, sessions: &[SessionSummary], query: &Query, kinds: &KindSet, opts: &SearchOptions) -> Result<(Vec<SessionHits>, usize)> {
    let wanted: HashMap<&str, usize> = sessions.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
    let text_kinds: Vec<TextKind> = kinds.kinds().into_iter().filter_map(Kind::text_kind).collect();
    let mut rows: HashMap<i64, Row> = HashMap::new();
    for (t, term) in query.terms().iter().enumerate() {
        for alt in &term.alternatives {
            rows_with(db, alt, &text_kinds, &mut |id, session_id, kind, offset, ts| {
                if let Some(&session) = wanted.get(session_id) {
                    rows.entry(id).or_insert(Row { id, session, kind, offset, ts, mask: 0 }).mask |= 1 << t;
                }
            })?;
        }
    }
    let mut per_session: HashMap<usize, Vec<Row>> = HashMap::new();
    for row in rows.into_values() {
        per_session.entry(row.session).or_default().push(row);
    }
    let mut results: Vec<(SessionHits, Vec<Row>)> = Vec::new();
    for (session, found) in per_session {
        let items: Vec<ItemRef> = found.iter().map(Row::item).collect();
        let Some((counts, term_hits)) = tally(&items, query, kinds) else { continue };
        let s = &sessions[session];
        let hits = SessionHits { session_id: s.id.clone(), updated_at: s.updated_at, counts, term_hits, turns: 0, snippets: Vec::new() };
        results.push((hits, found));
    }
    results.sort_by(|a, b| opts.sort.order(&a.0, &b.0));
    let total = results.len();
    results.truncate(shown(opts.limit, total));
    let mut text_of = db.conn().prepare_cached("SELECT text FROM messages WHERE id = ?1")?;
    let mut prompts_of =
        db.conn().prepare_cached("SELECT byte_offset FROM messages WHERE session_id = ?1 AND kind = 'prompt' ORDER BY byte_offset")?;
    let mut out = Vec::with_capacity(results.len());
    for (mut hits, found) in results {
        let prompts: Vec<i64> = prompts_of.query_map([&hits.session_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        hits.turns = prompts.len() as u32;
        let items: Vec<ItemRef> = found.iter().map(Row::item).collect();
        for i in pick(&items, opts.snippets) {
            let (row, item) = (&found[i], &items[i]);
            let text: String = text_of.query_row([row.id], |r| r.get(0))?;
            // the prompts up to the item, itself included
            let turn = prompts.partition_point(|&p| p <= item.offset as i64) as u32;
            hits.snippets.push(Hit {
                kind: item.kind,
                tool: None,
                turn: (turn > 0).then_some(turn),
                ts: (row.ts != 0).then_some(row.ts),
                offset: item.offset,
                terms: indexes(item.mask),
                text: query.matcher().snippet(&text, SNIPPET_BEFORE, SNIPPET_AFTER),
            });
        }
        out.push(hits);
    }
    Ok((out, total))
}

/// Every indexed row of these kinds whose text contains `alt` (case-insensitive). Three
/// characters or more go through the trigram index. Shorter ones scan the rows: `LIKE` when it
/// folds case the way the other paths do (ASCII and caseless characters), else a regex in Rust.
fn rows_with(db: &Db, alt: &str, kinds: &[TextKind], each: &mut dyn FnMut(i64, &str, Kind, u64, i64)) -> Result<()> {
    if kinds.is_empty() {
        return Ok(());
    }
    let marks = kinds.iter().map(|k| format!("'{}'", k.as_str())).collect::<Vec<_>>().join(",");
    let short = alt.chars().count() < 3;
    let in_rust = short && has_non_ascii_case(alt);
    let sql = if !short {
        format!(
            "SELECT m.id, m.session_id, m.kind, m.byte_offset, m.ts FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid
             WHERE messages_fts MATCH ?1 AND m.kind IN ({marks})"
        )
    } else if !in_rust {
        format!(
            "SELECT id, session_id, kind, byte_offset, ts FROM messages
             WHERE is_meta = 0 AND kind IN ({marks}) AND text LIKE ?1 ESCAPE '\\'"
        )
    } else {
        format!("SELECT id, session_id, kind, byte_offset, ts, text FROM messages WHERE is_meta = 0 AND kind IN ({marks})")
    };
    let args: Vec<String> = if !short {
        vec![format!("\"{}\"", alt.replace('"', "\"\""))]
    } else if !in_rust {
        vec![format!("%{}%", super::fts::escape_like(alt))]
    } else {
        Vec::new()
    };
    let re = in_rust.then(|| regex::RegexBuilder::new(&regex::escape(alt)).case_insensitive(true).build().expect("escaped text compiles"));
    let mut stmt = db.conn().prepare_cached(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(args))?;
    while let Some(r) = rows.next()? {
        if let Some(re) = &re
            && !re.is_match(r.get_ref(5)?.as_str()?)
        {
            continue;
        }
        let Some(kind) = TextKind::parse(r.get_ref(2)?.as_str()?) else { continue };
        each(r.get(0)?, r.get_ref(1)?.as_str()?, Kind::from_text_kind(kind), r.get::<_, i64>(3)? as u64, r.get(4)?);
    }
    Ok(())
}
