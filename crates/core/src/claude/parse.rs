//! Streaming transcript parser. Bytes go in (whole file or an appended tail), searchable
//! `MessageRow`s, one `UsageRow` per API response and a `SessionDelta` (aggregate fields) come out.
//!
//! Only user / assistant text and a handful of metadata records are materialised; tool results,
//! attachments and file-history records (98% of the bytes) are skipped after a byte-level look at
//! the line prefix. Prompt-counting rules follow the reference script's `human_prompt_text`.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use memchr::{memchr_iter, memmem};
use serde::Deserialize;
use serde_json::value::RawValue;

use super::clean::clean_prompt;
use crate::format::parse_ts;
use crate::model::{
    MessageRow, Role, SessionSummary, TITLE_RANK_AGENT_NAME, TITLE_RANK_AI, TITLE_RANK_CUSTOM,
    TITLE_RANK_SUMMARY, UsageRow,
};
use crate::pricing;

/// Model name Claude Code puts on records it writes itself (interrupts, API errors).
const SYNTHETIC_MODEL: &str = "<synthetic>";

/// Result of feeding bytes: the messages and API responses found, and how many bytes were
/// consumed (up to and including the last newline; a partial last line waits for the next feed).
#[derive(Debug, Default)]
pub struct Feed {
    pub messages: Vec<MessageRow>,
    /// One row per response within this feed. A response whose records straddle two feeds shows
    /// up in both, with the same `msg_id`: whoever keeps the rows must dedupe on it.
    pub usage: Vec<UsageRow>,
    pub consumed: u64,
}

/// A fully parsed transcript file.
#[derive(Debug)]
pub struct ParsedTranscript {
    pub summary: SessionSummary,
    pub messages: Vec<MessageRow>,
    pub usage: Vec<UsageRow>,
    pub consumed: u64,
}

/// Aggregate facts gathered from a chunk of records, applied onto a `SessionSummary` so that a
/// full parse and a sequence of incremental parses end up identical.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionDelta {
    pub first_prompt: Option<(String, Option<i64>)>,
    pub last_prompt: Option<String>,
    pub last_prompt_record: Option<String>,
    pub last_message: Option<String>,
    pub title: Option<(u8, String)>,
    pub user_msg_count: u32,
    pub assistant_msg_count: u32,
    pub tool_use_count: u32,
    pub tool_counts: BTreeMap<String, u32>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub cc_version: Option<String>,
    pub entrypoint: Option<String>,
    pub permission_mode: Option<String>,
    pub is_background: bool,
    pub total_cost_usd: Option<f64>,
    pub duration_ms: Option<i64>,
    pub lines_added: Option<i64>,
    pub lines_removed: Option<i64>,
    pub pr_url: Option<String>,
    pub files_touched: BTreeSet<String>,
    pub models: BTreeSet<String>,
    /// Prompt tokens and model of the last main-thread API call.
    pub context_tokens: Option<u64>,
    pub context_model: Option<String>,
    /// Largest prompt of any main-thread API call.
    pub context_peak: Option<u64>,
}

impl SessionDelta {
    /// Merge this chunk into a session: counts add up, "first" fields are only set once,
    /// "last" fields are overwritten, "max" fields keep the larger value.
    pub fn apply(&self, s: &mut SessionSummary) {
        if s.first_prompt.is_empty()
            && let Some((prompt, ts)) = &self.first_prompt
        {
            s.first_prompt = prompt.clone();
            s.started_at = *ts;
        }
        if let Some(p) = &self.last_prompt {
            s.last_prompt = Some(p.clone());
        } else if s.last_prompt.is_none()
            && let Some(p) = &self.last_prompt_record
        {
            s.last_prompt = Some(p.clone());
        }
        if let Some(m) = &self.last_message {
            s.last_message = Some(m.clone());
        }
        if let Some((rank, title)) = &self.title
            && *rank >= s.title_rank
        {
            s.title = Some(title.clone());
            s.title_rank = *rank;
        }
        s.user_msg_count += self.user_msg_count;
        s.assistant_msg_count += self.assistant_msg_count;
        s.tool_use_count += self.tool_use_count;
        for (name, n) in &self.tool_counts {
            *s.tool_counts.entry(name.clone()).or_default() += n;
        }
        if s.cwd.is_none() {
            s.cwd = self.cwd.clone();
        }
        if s.entrypoint.is_none() {
            s.entrypoint = self.entrypoint.clone();
        }
        if self.git_branch.is_some() {
            s.git_branch = self.git_branch.clone();
        }
        if self.cc_version.is_some() {
            s.cc_version = self.cc_version.clone();
        }
        if self.permission_mode.is_some() {
            s.permission_mode = self.permission_mode.clone();
        }
        s.is_background |= self.is_background;
        if self.total_cost_usd.is_some() {
            s.total_cost_usd = self.total_cost_usd;
            s.duration_ms = self.duration_ms;
            s.lines_added = self.lines_added;
            s.lines_removed = self.lines_removed;
        }
        if self.pr_url.is_some() {
            s.pr_url = self.pr_url.clone();
        }
        s.files_touched.extend(self.files_touched.iter().cloned());
        s.models.extend(self.models.iter().cloned());
        if self.context_tokens.is_some() {
            s.context_tokens = self.context_tokens;
            s.context_model = self.context_model.clone();
        }
        s.context_peak = s.context_peak.max(self.context_peak);
    }

    fn set_title(&mut self, rank: u8, title: Option<Cow<'_, str>>) {
        if let Some(t) = title
            && self.title.as_ref().is_none_or(|(r, _)| rank >= *r)
        {
            self.title = Some((rank, t.into_owned()));
        }
    }
}

/// Incremental parser for one transcript file.
#[derive(Debug, Default)]
pub struct TranscriptParser {
    delta: SessionDelta,
    usage_only: bool,
}

impl TranscriptParser {
    pub fn new(_session_id: &str) -> Self {
        Self::default()
    }

    /// A parser that only collects `UsageRow`s (subagent transcripts: their cost counts towards
    /// the parent session, nothing else of them is indexed).
    pub fn usage_only() -> Self {
        TranscriptParser { usage_only: true, ..Self::default() }
    }

    pub fn delta(&self) -> &SessionDelta {
        &self.delta
    }

    pub fn into_delta(self) -> SessionDelta {
        self.delta
    }

    /// Parse every complete line in `bytes`; `base_offset` is where `bytes` starts in the file.
    pub fn feed(&mut self, bytes: &[u8], base_offset: u64) -> Feed {
        let mut feed = Feed::default();
        let mut usage = UsageSink::default();
        let mut start = 0;
        for nl in memchr_iter(b'\n', bytes) {
            let mut line = &bytes[start..nl];
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            if !line.is_empty() {
                self.process_line(line, base_offset + start as u64, &mut feed.messages, &mut usage);
            }
            start = nl + 1;
        }
        feed.usage = usage.rows;
        feed.consumed = start as u64;
        feed
    }

    fn process_line(&mut self, line: &[u8], offset: u64, out: &mut Vec<MessageRow>, usage: &mut UsageSink) {
        match classify(line) {
            LineKind::Skip => return,
            LineKind::User if self.usage_only || !user_line_may_have_text(line) => return,
            LineKind::Meta if self.usage_only => return,
            _ => {}
        }
        let Ok(rec) = serde_json::from_slice::<RawRecord>(line) else { return };
        if self.usage_only {
            if rec.kind.as_deref() == Some("assistant") {
                self.on_usage(&rec, usage);
            }
            return;
        }
        match rec.kind.as_deref() {
            Some("user") => self.on_user(&rec, offset, line.len() as u64, out),
            Some("assistant") => {
                self.on_usage(&rec, usage);
                self.on_assistant(&rec, offset, line.len() as u64, out);
            }
            Some("last-prompt") => {
                if let Some(p) = &rec.last_prompt {
                    self.delta.last_prompt_record = Some(p.to_string());
                }
            }
            Some("custom-title") => self.delta.set_title(TITLE_RANK_CUSTOM, rec.custom_title),
            Some("agent-name") => self.delta.set_title(TITLE_RANK_AGENT_NAME, rec.agent_name),
            Some("ai-title") => self.delta.set_title(TITLE_RANK_AI, rec.ai_title),
            Some("summary") => self.delta.set_title(TITLE_RANK_SUMMARY, rec.summary),
            Some("cost-state") => {
                self.delta.total_cost_usd = rec.total_cost_usd;
                self.delta.duration_ms = rec.total_duration;
                self.delta.lines_added = rec.lines_added;
                self.delta.lines_removed = rec.lines_removed;
            }
            Some("pr-link") => {
                if let Some(url) = rec.pr_url {
                    self.delta.pr_url = Some(url.into_owned());
                }
            }
            Some("permission-mode") => {
                if let Some(mode) = rec.permission_mode {
                    self.delta.permission_mode = Some(mode.into_owned());
                }
            }
            Some("file-history-delta") => {
                if let Some(path) = rec.tracking_path {
                    self.delta.files_touched.insert(path.into_owned());
                }
            }
            _ => {}
        }
    }

    fn note_env(&mut self, rec: &RawRecord) {
        if self.delta.cwd.is_none() {
            self.delta.cwd = rec.cwd.as_deref().map(str::to_string);
        }
        if self.delta.entrypoint.is_none() {
            self.delta.entrypoint = rec.entrypoint.as_deref().map(str::to_string);
        }
        if let Some(b) = &rec.git_branch {
            self.delta.git_branch = Some(b.to_string());
        }
        if let Some(v) = &rec.version {
            self.delta.cc_version = Some(v.to_string());
        }
        if let Some(m) = &rec.permission_mode {
            self.delta.permission_mode = Some(m.to_string());
        }
        if rec.session_kind.as_deref() == Some("bg") {
            self.delta.is_background = true;
        }
    }

    fn on_user(&mut self, rec: &RawRecord, offset: u64, len: u64, out: &mut Vec<MessageRow>) {
        let Some(content) = rec.message.as_ref().and_then(|m| m.content).map(parse_content) else { return };
        let Some(text) = user_text(&content) else { return };
        let ts = rec.timestamp.as_deref().and_then(parse_ts);
        if is_human_prompt(rec, &text) {
            self.delta.user_msg_count += 1;
            let cleaned = clean_prompt(&text);
            if !cleaned.is_empty() {
                if self.delta.first_prompt.is_none() {
                    self.delta.first_prompt = Some((cleaned.clone(), ts));
                }
                self.delta.last_prompt = Some(cleaned);
            }
        }
        out.push(MessageRow {
            uuid: rec.uuid.as_deref().unwrap_or("").to_string(),
            parent_uuid: rec.parent_uuid.as_deref().map(str::to_string),
            role: Role::User,
            ts: ts.unwrap_or(0),
            text,
            is_meta: rec.is_meta,
            byte_offset: offset,
            byte_len: len,
            model: None,
            input_tokens: None,
            output_tokens: None,
        });
        self.note_env(rec);
    }

    /// Every assistant record carries its response's usage, text or not.
    fn on_usage(&mut self, rec: &RawRecord, sink: &mut UsageSink) {
        let Some(row) = usage_row(rec) else { return };
        if !rec.is_sidechain {
            let prompt = row.prompt_tokens();
            self.delta.context_tokens = Some(prompt);
            self.delta.context_model = Some(row.model.clone());
            self.delta.context_peak = self.delta.context_peak.max(Some(prompt));
        }
        sink.push(row);
    }

    fn on_assistant(&mut self, rec: &RawRecord, offset: u64, len: u64, out: &mut Vec<MessageRow>) {
        let Some(msg) = rec.message.as_ref() else { return };
        if let Some(Content::Blocks(blocks)) = msg.content.map(parse_content) {
            let mut texts: Vec<String> = Vec::new();
            for block in &blocks {
                match block.kind.as_deref() {
                    Some("text") => {
                        if let Some(t) = &block.text {
                            texts.push(t.to_string());
                        }
                    }
                    Some("tool_use") => {
                        self.delta.tool_use_count += 1;
                        let name = block.name.as_deref().unwrap_or("?").to_string();
                        *self.delta.tool_counts.entry(name).or_default() += 1;
                    }
                    _ => {}
                }
            }
            if !texts.is_empty() {
                let text = texts.join("\n");
                self.delta.assistant_msg_count += 1;
                self.delta.last_message = Some(text.clone());
                out.push(MessageRow {
                    uuid: rec.uuid.as_deref().unwrap_or("").to_string(),
                    parent_uuid: rec.parent_uuid.as_deref().map(str::to_string),
                    role: Role::Assistant,
                    ts: rec.timestamp.as_deref().and_then(parse_ts).unwrap_or(0),
                    text,
                    is_meta: false,
                    byte_offset: offset,
                    byte_len: len,
                    model: msg.model.as_deref().map(str::to_string),
                    input_tokens: msg.usage.as_ref().and_then(|u| u.input_tokens),
                    output_tokens: msg.usage.as_ref().and_then(|u| u.output_tokens),
                });
            }
        }
        if let Some(model) = msg.model.as_deref().filter(|m| *m != SYNTHETIC_MODEL) {
            self.delta.models.insert(model.to_string());
        }
        self.note_env(rec);
    }
}

/// Usage rows of one feed, deduplicated on `msg_id`: the last record of a response wins (they
/// all carry the same usage), but the row keeps its position and the first record's timestamp.
#[derive(Default)]
struct UsageSink {
    rows: Vec<UsageRow>,
    by_id: HashMap<String, usize>,
}

impl UsageSink {
    fn push(&mut self, row: UsageRow) {
        match self.by_id.get(&row.msg_id) {
            Some(&ix) => {
                let ts = self.rows[ix].ts;
                self.rows[ix] = UsageRow { ts, ..row };
            }
            None => {
                self.by_id.insert(row.msg_id.clone(), self.rows.len());
                self.rows.push(row);
            }
        }
    }
}

/// The priced usage of an assistant record; `None` for records Claude Code wrote itself and
/// for records without usage or any id to dedupe on.
fn usage_row(rec: &RawRecord) -> Option<UsageRow> {
    let msg = rec.message.as_ref()?;
    let model = msg.model.as_deref().filter(|m| *m != SYNTHETIC_MODEL)?;
    let u = msg.usage.as_ref()?;
    let msg_id = [msg.id.as_deref(), rec.request_id.as_deref(), rec.uuid.as_deref()]
        .into_iter()
        .flatten()
        .find(|id| !id.is_empty())?;
    let cache_write = u.cache_creation_input_tokens.unwrap_or(0);
    // records written before the 5m / 1h split are all 5-minute writes
    let (cache_write_5m, cache_write_1h) = match &u.cache_creation {
        Some(c) => (c.ephemeral_5m_input_tokens.unwrap_or(0), c.ephemeral_1h_input_tokens.unwrap_or(0)),
        None => (cache_write, 0),
    };
    let mut row = UsageRow {
        msg_id: msg_id.to_string(),
        model: model.to_string(),
        ts: rec.timestamp.as_deref().and_then(parse_ts).unwrap_or(0),
        input: u.input_tokens.unwrap_or(0).max(0) as u64,
        cache_write_5m,
        cache_write_1h,
        cache_read: u.cache_read_input_tokens.unwrap_or(0),
        output: u.output_tokens.unwrap_or(0).max(0) as u64,
        web_search: u.server_tool_use.as_ref().and_then(|s| s.web_search_requests).unwrap_or(0),
        fast: u.speed.as_deref() == Some("fast"),
        cost_usd: None,
    };
    row.cost_usd = pricing::cost(&row);
    Some(row)
}

/// Parse a whole transcript file. The session id is the file stem, the project key its directory.
pub fn parse_transcript(path: &Path) -> Result<ParsedTranscript> {
    let session_id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let project_key = path.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap_or("").to_string();
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let meta = file.metadata()?;
    let bytes = map_file(&file, meta.len())?;
    let mut parser = TranscriptParser::new(&session_id);
    let feed = parser.feed(&bytes, 0);
    let mut summary = SessionSummary::transcript(&session_id, &project_key, path);
    parser.delta().apply(&mut summary);
    summary.set_usage(&feed.usage);
    summary.updated_at = mtime_ms(&meta);
    summary.file_size = Some(meta.len());
    Ok(ParsedTranscript { summary, messages: feed.messages, usage: feed.usage, consumed: feed.consumed })
}

/// Memory-map a file (an empty file maps to an empty buffer, which mmap refuses).
pub fn map_file(file: &File, len: u64) -> Result<Mapped> {
    if len == 0 {
        return Ok(Mapped::Empty);
    }
    // SAFETY: transcripts are append-only; a concurrent append only grows the file past `len`.
    let map = unsafe { memmap2::Mmap::map(file)? };
    Ok(Mapped::Map(map))
}

pub enum Mapped {
    Empty,
    Map(memmap2::Mmap),
}

impl std::ops::Deref for Mapped {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Mapped::Empty => &[],
            Mapped::Map(m) => m,
        }
    }
}

pub fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// Byte-level line classification (no JSON parsing)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineKind {
    User,
    Assistant,
    /// A metadata record worth parsing (titles, cost, last prompt, ...).
    Meta,
    /// Attachments, hooks, file history, modes: never needed.
    Skip,
    /// Could not tell from the prefix; parse the JSON to find out.
    Unknown,
}

const PREFIX_LEN: usize = 512;
const META_TYPES: &[&str] = &[
    "last-prompt",
    "custom-title",
    "ai-title",
    "agent-name",
    "summary",
    "cost-state",
    "pr-link",
    "permission-mode",
    "file-history-delta",
];

/// Classify a record from its first bytes. Verified against every line on the reference machine:
/// the top-level `type` (or, for assistant records where `message` comes first, the `role`)
/// is always within the first 512 bytes.
pub(crate) fn classify(line: &[u8]) -> LineKind {
    let prefix = &line[..line.len().min(PREFIX_LEN)];
    if memmem::find(prefix, b"\"attachment\":").is_some() {
        return LineKind::Skip;
    }
    let Some(kind) = json_string_after(prefix, b"\"type\":") else { return LineKind::Unknown };
    match kind {
        b"user" => LineKind::User,
        b"assistant" => LineKind::Assistant,
        b"message" => match json_string_after(prefix, b"\"role\":") {
            Some(b"user") => LineKind::User,
            Some(b"assistant") => LineKind::Assistant,
            _ => LineKind::Unknown,
        },
        other => {
            if META_TYPES.iter().any(|t| t.as_bytes() == other) {
                LineKind::Meta
            } else {
                LineKind::Skip
            }
        }
    }
}

/// The string value following `key` (e.g. `"type":`), tolerating whitespace after the colon.
fn json_string_after<'a>(hay: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let at = memmem::find(hay, key)? + key.len();
    let rest = &hay[at..];
    let skip = rest.iter().take_while(|b| b.is_ascii_whitespace()).count();
    let rest = rest.get(skip..)?;
    if rest.first() != Some(&b'"') {
        return None;
    }
    let end = memchr::memchr(b'"', &rest[1..])?;
    Some(&rest[1..1 + end])
}

/// A user record only matters when it carries text: string content, or an array with a text
/// block. Tool-result-only records (the bulk of the corpus) are skipped without parsing.
fn user_line_may_have_text(line: &[u8]) -> bool {
    let Some(at) = memmem::find(line, b"\"content\":") else { return false };
    let rest = &line[at + 10..];
    let skip = rest.iter().take_while(|b| b.is_ascii_whitespace()).count();
    match rest.get(skip) {
        Some(b'"') => true,
        Some(b'[') => memmem::find(rest, b"\"type\":\"text\"").is_some() || memmem::find(rest, b"\"type\": \"text\"").is_some(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Borrowed record types (no `deny_unknown_fields`: the schema grows every month)

#[derive(Deserialize)]
pub(crate) struct RawRecord<'a> {
    #[serde(rename = "type", borrow)]
    pub kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub message: Option<RawMessage<'a>>,
    #[serde(borrow)]
    pub uuid: Option<Cow<'a, str>>,
    #[serde(rename = "requestId", borrow)]
    pub request_id: Option<Cow<'a, str>>,
    #[serde(rename = "parentUuid", borrow)]
    pub parent_uuid: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub timestamp: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub cwd: Option<Cow<'a, str>>,
    #[serde(rename = "gitBranch", borrow)]
    pub git_branch: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub version: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub entrypoint: Option<Cow<'a, str>>,
    #[serde(rename = "isSidechain", default)]
    pub is_sidechain: bool,
    #[serde(rename = "isMeta", default)]
    pub is_meta: bool,
    #[serde(rename = "sessionKind", borrow)]
    pub session_kind: Option<Cow<'a, str>>,
    #[serde(rename = "permissionMode", borrow)]
    pub permission_mode: Option<Cow<'a, str>>,
    #[serde(rename = "lastPrompt", borrow)]
    pub last_prompt: Option<Cow<'a, str>>,
    #[serde(rename = "customTitle", borrow)]
    pub custom_title: Option<Cow<'a, str>>,
    #[serde(rename = "aiTitle", borrow)]
    pub ai_title: Option<Cow<'a, str>>,
    #[serde(rename = "agentName", borrow)]
    pub agent_name: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub summary: Option<Cow<'a, str>>,
    #[serde(rename = "totalCostUSD")]
    pub total_cost_usd: Option<f64>,
    #[serde(rename = "totalDuration")]
    pub total_duration: Option<i64>,
    #[serde(rename = "totalLinesAdded")]
    pub lines_added: Option<i64>,
    #[serde(rename = "totalLinesRemoved")]
    pub lines_removed: Option<i64>,
    #[serde(rename = "prUrl", borrow)]
    pub pr_url: Option<Cow<'a, str>>,
    #[serde(rename = "trackingPath", borrow)]
    pub tracking_path: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
pub(crate) struct RawMessage<'a> {
    #[serde(borrow)]
    pub id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub content: Option<&'a RawValue>,
    #[serde(borrow)]
    pub model: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub usage: Option<RawUsage<'a>>,
}

#[derive(Deserialize)]
pub(crate) struct RawUsage<'a> {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_creation: Option<RawCacheCreation>,
    pub server_tool_use: Option<RawServerToolUse>,
    #[serde(borrow)]
    pub speed: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
pub(crate) struct RawCacheCreation {
    pub ephemeral_5m_input_tokens: Option<u64>,
    pub ephemeral_1h_input_tokens: Option<u64>,
}

#[derive(Deserialize)]
pub(crate) struct RawServerToolUse {
    pub web_search_requests: Option<u64>,
}

/// One content block; which fields are present depends on `kind`.
#[derive(Deserialize)]
pub(crate) struct RawBlock<'a> {
    #[serde(rename = "type", borrow)]
    pub kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub text: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub thinking: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub name: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub input: Option<&'a RawValue>,
    #[serde(borrow)]
    pub content: Option<&'a RawValue>,
    #[serde(borrow)]
    pub id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    pub tool_use_id: Option<Cow<'a, str>>,
    pub is_error: Option<bool>,
    #[serde(borrow)]
    pub source: Option<RawSource<'a>>,
}

#[derive(Deserialize)]
pub(crate) struct RawSource<'a> {
    #[serde(borrow)]
    pub data: Option<&'a RawValue>,
}

pub(crate) enum Content<'a> {
    Text(Cow<'a, str>),
    Blocks(Vec<RawBlock<'a>>),
    Other,
}

#[derive(Deserialize)]
struct BorrowedStr<'a>(#[serde(borrow)] Cow<'a, str>);

/// `message.content` is either a string or an array of blocks. Elements that are not objects
/// are skipped, like the reference script does.
pub(crate) fn parse_content(raw: &RawValue) -> Content<'_> {
    let s = raw.get().trim_start();
    match s.as_bytes().first() {
        Some(b'"') => serde_json::from_str::<BorrowedStr>(s).map(|b| Content::Text(b.0)).unwrap_or(Content::Other),
        Some(b'[') => match serde_json::from_str::<Vec<&RawValue>>(s) {
            Ok(items) => Content::Blocks(
                items.into_iter().filter_map(|item| serde_json::from_str::<RawBlock>(item.get()).ok()).collect(),
            ),
            Err(_) => Content::Other,
        },
        _ => Content::Other,
    }
}

/// Text of a user record: the string content, or its text blocks joined by newlines.
/// `None` for records without text (tool results, images).
pub(crate) fn user_text(content: &Content) -> Option<String> {
    match content {
        Content::Text(t) => Some(t.to_string()),
        Content::Blocks(blocks) => {
            let parts: Vec<&str> = blocks
                .iter()
                .filter(|b| b.kind.as_deref() == Some("text"))
                .map(|b| b.text.as_deref().unwrap_or(""))
                .collect();
            if parts.is_empty() { None } else { Some(parts.join("\n")) }
        }
        Content::Other => None,
    }
}

/// `human_prompt_text` rule: not a subagent line, not harness-injected, not an image carrier.
pub(crate) fn is_human_prompt(rec: &RawRecord, text: &str) -> bool {
    !rec.is_sidechain && !rec.is_meta && !text.starts_with("[Image: source:")
}
