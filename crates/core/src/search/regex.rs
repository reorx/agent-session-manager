//! Regex search over raw transcripts: the reference script's search mode. Every user /
//! assistant record is split into "pieces" (prompt text, reply text, thinking, tool_use input,
//! tool_result output); a piece with a match yields one `Match` with a context snippet.

use std::borrow::Cow;
use std::fs::File;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::LazyLock;

use anyhow::Result;
use memchr::memchr_iter;
use rayon::prelude::*;
use regex::{Regex, RegexBuilder};
use serde_json::{Map, Value};

use super::Highlighted;
use super::pyjson::stringify;
use crate::claude::discover::{TranscriptFile, discover_all};
use crate::claude::parse::{Content, LineKind, RawRecord, classify, map_file, parse_content};
use crate::claude::paths::ClaudeDir;
use crate::format::parse_ts;

/// Files scanned per rayon batch; the limit is checked between batches, so scanning stops
/// early like the sequential script does.
const BATCH: usize = 32;
/// Tool input fields longer than this are cut around the match.
pub const MAX_FIELD_CHARS: usize = 300;
/// Commands with a longer line are reformatted by `shfmt` when it is installed.
const WRAP_LEN: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoleSet {
    pub user: bool,
    pub assistant: bool,
    pub tool: bool,
}

impl RoleSet {
    pub fn all() -> Self {
        RoleSet { user: true, assistant: true, tool: true }
    }

    pub fn contains(&self, role: MatchRole) -> bool {
        match role {
            MatchRole::User => self.user,
            MatchRole::Assistant => self.assistant,
            MatchRole::Tool => self.tool,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchRole {
    User,
    Assistant,
    Tool,
}

impl MatchRole {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchRole::User => "user",
            MatchRole::Assistant => "assistant",
            MatchRole::Tool => "tool",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RegexOptions {
    pub roles: RoleSet,
    /// Keep only pieces whose label contains one of these (case-insensitive); prompts and
    /// replies have no label, so any label filter leaves them out.
    pub labels: Vec<String>,
    pub case_sensitive: bool,
    /// Characters of context on each side of a match.
    pub context: usize,
    /// Stop after this many matches.
    pub limit: usize,
}

impl Default for RegexOptions {
    fn default() -> Self {
        RegexOptions { roles: RoleSet::all(), labels: Vec::new(), case_sensitive: false, context: 80, limit: 200 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolLineKind {
    Description,
    Command,
    Field(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolLine {
    pub kind: ToolLineKind,
    pub text: Highlighted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Snippet {
    Text(Highlighted),
    /// A tool call: description, command (in full), then the other fields.
    ToolUse(Vec<ToolLine>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub role: MatchRole,
    pub timestamp_ms: Option<i64>,
    /// `thinking`, `tool_use:<name>`, `tool_result`, or empty.
    pub detail: String,
    pub snippet: Snippet,
}

#[derive(Clone, Debug)]
pub struct FileMatches {
    pub project_key: String,
    pub session_id: String,
    pub path: PathBuf,
    pub mtime_ms: i64,
    /// Matches found in the file (`matches` may hold fewer once the limit is reached).
    pub match_count: usize,
    pub matches: Vec<Match>,
}

#[derive(Clone, Debug, Default)]
pub struct SearchOutcome {
    pub files: Vec<FileMatches>,
    /// Matches found in the files scanned before the limit stopped the scan.
    pub total: usize,
    /// Transcripts in range (not all of them are scanned when the limit is reached).
    pub scanned: usize,
    /// Matches kept in `files` (at most `limit`).
    pub shown: usize,
}

/// Transcripts modified within the last `days` (`None` / 0 = all) whose project name contains
/// `project` (case-insensitive), newest first.
pub fn list_transcripts(claude: &ClaudeDir, favorites: Option<&ClaudeDir>, days: Option<f64>, project: Option<&str>) -> Result<Vec<TranscriptFile>> {
    let cutoff = match days {
        Some(d) if d > 0.0 => chrono::Utc::now().timestamp_millis() - (d * 86_400_000.0) as i64,
        _ => i64::MIN,
    };
    let needle = project.map(|p| p.to_lowercase()).filter(|p| !p.is_empty());
    let mut files: Vec<TranscriptFile> = discover_all(claude, favorites)?.sessions
        .into_iter()
        .filter(|f| f.mtime_ms >= cutoff)
        .filter(|f| needle.as_ref().is_none_or(|n| f.project_key.to_lowercase().contains(n)))
        .collect();
    files.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms).then_with(|| a.path.cmp(&b.path)));
    Ok(files)
}

/// Compile the query; a query that is not a valid regex is matched as literal text.
pub fn compile_query(query: &str, case_sensitive: bool) -> Result<Regex> {
    let build = |pattern: &str| RegexBuilder::new(pattern).case_insensitive(!case_sensitive).build();
    Ok(match build(query) {
        Ok(re) => re,
        Err(_) => build(&regex::escape(query))?,
    })
}

/// Scan `files` in order (batches in parallel) and stop once `opts.limit` matches are kept.
pub fn search(query: &str, files: &[TranscriptFile], opts: &RegexOptions) -> SearchOutcome {
    let re = compile_query(query, opts.case_sensitive).expect("escaped query always compiles");
    let mut out = SearchOutcome { scanned: files.len(), ..Default::default() };
    for batch in files.chunks(BATCH) {
        let results: Vec<Vec<Match>> = batch.par_iter().map(|f| search_path(f, &re, opts, usize::MAX).matches).collect();
        for (f, mut matches) in batch.iter().zip(results) {
            if matches.is_empty() {
                continue;
            }
            let match_count = matches.len();
            out.total += match_count;
            matches.truncate(opts.limit.saturating_sub(out.shown));
            out.shown += matches.len();
            out.files.push(FileMatches {
                project_key: f.project_key.clone(),
                session_id: f.session_id.clone(),
                path: f.path.clone(),
                mtime_ms: f.mtime_ms,
                match_count,
                matches,
            });
            if out.shown >= opts.limit {
                return out;
            }
        }
    }
    out
}

/// What a scan for sessions (rather than for matches) found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionScan {
    /// Sessions with at least one match that were emitted.
    pub sessions: usize,
    /// Matches in those sessions.
    pub matches: usize,
    /// Transcripts read before the scan ended.
    pub scanned: usize,
    /// The session cap ended the scan; later transcripts may match as well.
    pub truncated: bool,
    pub cancelled: bool,
}

/// The interactive variant of `search`: every transcript with a match is reported with its
/// match count and its first match only, batch by batch through `emit`, until `max_sessions`
/// sessions matched or `cancelled` turns true (checked per file). `opts.limit` does not apply.
pub fn scan_sessions(
    query: &str,
    files: &[TranscriptFile],
    opts: &RegexOptions,
    max_sessions: usize,
    cancelled: &(dyn Fn() -> bool + Sync),
    emit: &mut dyn FnMut(Vec<FileMatches>),
) -> SessionScan {
    let re = compile_query(query, opts.case_sensitive).expect("escaped query always compiles");
    let mut out = SessionScan::default();
    for batch in files.chunks(BATCH) {
        let results: Vec<Option<Found>> = batch.par_iter().map(|f| if cancelled() { None } else { Some(search_path(f, &re, opts, 1)) }).collect();
        if cancelled() || results.iter().any(Option::is_none) {
            out.cancelled = true;
            return out;
        }
        out.scanned += batch.len();
        let mut hits: Vec<FileMatches> = batch
            .iter()
            .zip(results.into_iter().flatten())
            .filter(|(_, found)| found.count > 0)
            .map(|(f, found)| FileMatches {
                project_key: f.project_key.clone(),
                session_id: f.session_id.clone(),
                path: f.path.clone(),
                mtime_ms: f.mtime_ms,
                match_count: found.count,
                matches: found.matches,
            })
            .collect();
        let room = max_sessions.saturating_sub(out.sessions);
        let full = hits.len() >= room;
        out.truncated = hits.len() > room || (full && out.scanned < files.len());
        hits.truncate(room);
        out.sessions += hits.len();
        out.matches += hits.iter().map(|h| h.match_count).sum::<usize>();
        if !hits.is_empty() {
            emit(hits);
        }
        if full {
            return out;
        }
    }
    out
}

/// Matches of one transcript: all of them counted, the first `keep` built.
#[derive(Default)]
struct Found {
    matches: Vec<Match>,
    count: usize,
    keep: usize,
}

fn search_path(f: &TranscriptFile, re: &Regex, opts: &RegexOptions, keep: usize) -> Found {
    let none = Found::default;
    let Ok(handle) = File::open(&f.path) else { return none() };
    let Ok(len) = handle.metadata().map(|m| m.len()) else { return none() };
    let Ok(bytes) = map_file(&handle, len) else { return none() };
    find_in_bytes(&bytes, re, opts, keep)
}

/// Search one transcript's bytes; matches come back in file order.
pub fn search_bytes(bytes: &[u8], re: &Regex, opts: &RegexOptions) -> Vec<Match> {
    find_in_bytes(bytes, re, opts, usize::MAX).matches
}

fn find_in_bytes(bytes: &[u8], re: &Regex, opts: &RegexOptions, keep: usize) -> Found {
    let mut out = Found { keep, ..Default::default() };
    let mut start = 0;
    for nl in memchr_iter(b'\n', bytes) {
        let line = &bytes[start..nl];
        start = nl + 1;
        search_line(line, re, opts, &mut out);
    }
    if start < bytes.len() {
        search_line(&bytes[start..], re, opts, &mut out);
    }
    out
}

fn search_line(line: &[u8], re: &Regex, opts: &RegexOptions, out: &mut Found) {
    match classify(line) {
        LineKind::Skip | LineKind::Meta => return,
        LineKind::User if !opts.roles.user && !opts.roles.tool => return,
        LineKind::Assistant if !opts.roles.assistant && !opts.roles.tool => return,
        _ => {}
    }
    let Ok(rec) = serde_json::from_slice::<RawRecord>(line) else { return };
    let ts = rec.timestamp.as_deref().and_then(parse_ts);
    for piece in iter_texts(&rec, opts.roles.tool) {
        if !opts.roles.contains(piece.role) || piece.text.is_empty() {
            continue;
        }
        if !opts.labels.is_empty() {
            let detail = piece.detail.to_lowercase();
            if !opts.labels.iter().any(|l| detail.contains(&l.to_lowercase())) {
                continue;
            }
        }
        let Some(m) = re.find(&piece.text) else { continue };
        out.count += 1;
        if out.matches.len() >= out.keep {
            continue;
        }
        let snippet = match &piece.tool_input {
            Some(input) => Snippet::ToolUse(render_tool_use(input, re, opts.context)),
            None => Snippet::Text(make_snippet(&piece.text, m.range(), opts.context)),
        };
        out.matches.push(Match { role: piece.role, timestamp_ms: ts, detail: piece.detail, snippet });
    }
}

struct Piece<'a> {
    role: MatchRole,
    text: Cow<'a, str>,
    detail: String,
    tool_input: Option<Map<String, Value>>,
}

/// The reference script's `iter_texts`: (role, text, label, tool input) per content block.
fn iter_texts<'a>(rec: &RawRecord<'a>, want_tool: bool) -> Vec<Piece<'a>> {
    let mut pieces = Vec::new();
    let role = match rec.kind.as_deref() {
        Some("user") => MatchRole::User,
        Some("assistant") => MatchRole::Assistant,
        _ => return pieces,
    };
    let Some(content) = rec.message.as_ref().and_then(|m| m.content) else { return pieces };
    match parse_content(content) {
        Content::Text(text) => {
            if role == MatchRole::User {
                pieces.push(Piece { role, text, detail: String::new(), tool_input: None });
            }
        }
        Content::Blocks(blocks) => {
            for block in blocks {
                match block.kind.as_deref() {
                    Some("text") => {
                        pieces.push(Piece { role, text: block.text.unwrap_or_default(), detail: String::new(), tool_input: None });
                    }
                    Some("thinking") if role == MatchRole::Assistant => {
                        pieces.push(Piece { role, text: block.thinking.unwrap_or_default(), detail: "thinking".into(), tool_input: None });
                    }
                    Some("tool_use") if want_tool => {
                        let input = match block.input.and_then(|raw| serde_json::from_str::<Value>(raw.get()).ok()) {
                            Some(Value::Object(map)) => map,
                            Some(other) => Map::from_iter([("input".to_string(), other)]),
                            None => Map::new(),
                        };
                        let text = input.iter().map(|(k, v)| format!("{k}: {}", stringify(v))).collect::<Vec<_>>().join("\n");
                        let name = block.name.as_deref().unwrap_or("?");
                        pieces.push(Piece { role: MatchRole::Tool, text: Cow::Owned(text), detail: format!("tool_use:{name}"), tool_input: Some(input) });
                    }
                    Some("tool_result") if want_tool => {
                        let text = match block.content.and_then(|raw| serde_json::from_str::<Value>(raw.get()).ok()) {
                            Some(Value::String(s)) => s,
                            Some(Value::Array(items)) => items
                                .iter()
                                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                                .map(|b| b.get("text").and_then(Value::as_str).unwrap_or(""))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            _ => String::new(),
                        };
                        pieces.push(Piece { role: MatchRole::Tool, text: Cow::Owned(text), detail: "tool_result".into(), tool_input: None });
                    }
                    _ => {}
                }
            }
        }
        Content::Other => {}
    }
    pieces
}

/// `ctx` characters on each side of the match; `…` marks cut text and `⏎` a line break.
pub fn make_snippet(text: &str, m: Range<usize>, ctx: usize) -> Highlighted {
    let start = text[..m.start].char_indices().rev().nth(ctx.saturating_sub(1)).map(|(i, _)| i).unwrap_or(0);
    let start = if ctx == 0 { m.start } else { start };
    let end = text[m.end..].char_indices().nth(ctx).map(|(i, _)| m.end + i).unwrap_or(text.len());
    let mut out = Highlighted::default();
    if start > 0 {
        out.text.push('…');
    }
    out.text.push_str(&text[start..m.start].replace('\n', "⏎"));
    let hl_start = out.text.len();
    out.text.push_str(&text[m.clone()].replace('\n', "⏎"));
    out.ranges.push(hl_start..out.text.len());
    out.text.push_str(&text[m.end..end].replace('\n', "⏎"));
    if end < text.len() {
        out.text.push('…');
    }
    out
}

/// Every match highlighted, text otherwise untouched.
pub fn highlight_all(text: &str, re: &Regex) -> Highlighted {
    Highlighted { text: text.to_string(), ranges: re.find_iter(text).map(|m| m.range()).collect() }
}

/// The reference script's `render_tool_use`: description as `# ...` and command as `$ ...` in
/// full, other fields cut around the match when longer than `MAX_FIELD_CHARS`.
pub fn render_tool_use(input: &Map<String, Value>, re: &Regex, ctx: usize) -> Vec<ToolLine> {
    let mut lines = Vec::new();
    if let Some(desc) = input.get("description") {
        lines.push(ToolLine { kind: ToolLineKind::Description, text: highlight_all(&stringify(desc), re) });
    }
    if let Some(cmd) = input.get("command") {
        lines.push(ToolLine { kind: ToolLineKind::Command, text: highlight_all(&format_command(&stringify(cmd)), re) });
    }
    for (k, v) in input {
        if k == "description" || k == "command" {
            continue;
        }
        let s = stringify(v);
        let text = if s.chars().count() <= MAX_FIELD_CHARS {
            highlight_all(&s, re)
        } else if let Some(m) = re.find(&s) {
            make_snippet(&s, m.range(), ctx)
        } else {
            let mut cut: String = s.chars().take(MAX_FIELD_CHARS).collect::<String>().replace('\n', "⏎");
            cut.push('…');
            Highlighted::plain(cut)
        };
        lines.push(ToolLine { kind: ToolLineKind::Field(k.clone()), text });
    }
    lines
}

static SHFMT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("shfmt")).find(|p| p.is_file())
});

/// Reformat a shell command with `shfmt -i 2` when any line exceeds `WRAP_LEN` characters.
pub fn format_command(cmd: &str) -> String {
    let Some(shfmt) = SHFMT.as_ref() else { return cmd.to_string() };
    if cmd.lines().all(|l| l.chars().count() <= WRAP_LEN) {
        return cmd.to_string();
    }
    use std::io::Write;
    use std::process::{Command, Stdio};
    let child = Command::new(shfmt).args(["-i", "2"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = child else { return cmd.to_string() };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(cmd.as_bytes());
    }
    match child.wait_with_output() {
        Ok(out) if out.status.success() && !out.stdout.iter().all(|b| b.is_ascii_whitespace()) => {
            String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string()
        }
        _ => cmd.to_string(),
    }
}
