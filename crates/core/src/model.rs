//! Data types shared across the crate: one row per session, one row per searchable message.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where a session row comes from: a transcript file, or only `history.jsonl`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    Transcript,
    History,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Transcript => "transcript",
            Source::History => "history",
        }
    }

    pub fn parse(s: &str) -> Source {
        if s == "history" { Source::History } else { Source::Transcript }
    }
}

/// Role of an indexed message (tool results and thinking are never indexed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            _ => None,
        }
    }
}

/// Which record set a title: higher wins, so incremental updates keep the best one.
pub const TITLE_RANK_SUMMARY: u8 = 1;
pub const TITLE_RANK_AI: u8 = 2;
pub const TITLE_RANK_AGENT_NAME: u8 = 3;
pub const TITLE_RANK_CUSTOM: u8 = 4;

/// Everything the table shows for one session. Mirrors the `sessions` table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    /// Session uuid, or a synthetic `history:<project>:<ts>` id when `id_known` is false.
    pub id: String,
    pub id_known: bool,
    /// Encoded project directory name, e.g. `-Users-me-Code-alpha`.
    pub project_key: String,
    /// Real project path (from `cwd` or history `project`).
    pub project_path: Option<String>,
    pub file_path: Option<PathBuf>,
    pub source: Source,
    pub transcript_missing: bool,
    pub favorite: bool,
    pub title: Option<String>,
    pub title_rank: u8,
    /// First human prompt (cleaned); empty when the session never got one.
    pub first_prompt: String,
    pub last_prompt: Option<String>,
    pub last_message: Option<String>,
    /// Timestamp of the first prompt, unix ms.
    pub started_at: Option<i64>,
    /// Transcript mtime, or the last history row, unix ms.
    pub updated_at: i64,
    pub git_branch: Option<String>,
    pub cwd: Option<String>,
    pub cc_version: Option<String>,
    pub entrypoint: Option<String>,
    pub permission_mode: Option<String>,
    pub is_background: bool,
    pub user_msg_count: u32,
    pub assistant_msg_count: u32,
    pub tool_use_count: u32,
    pub tool_counts: BTreeMap<String, u32>,
    pub subagent_count: u32,
    pub file_size: Option<u64>,
    pub total_cost_usd: Option<f64>,
    pub duration_ms: Option<i64>,
    pub lines_added: Option<i64>,
    pub lines_removed: Option<i64>,
    pub pr_url: Option<String>,
    pub files_touched: BTreeSet<String>,
    pub models: BTreeSet<String>,
    /// Priced from token usage (subagents included); `None` when no call could be priced.
    pub cost_usd: Option<f64>,
    /// Uncached input tokens, summed over every API response (subagents included).
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
    /// Prompt tokens of the last main-thread API call: what the context held at the end.
    pub context_tokens: Option<u64>,
    pub context_peak: Option<u64>,
    /// Model of that last call, which decides the context window.
    pub context_model: Option<String>,
    /// Models that showed up in the usage but have no price, so `cost_usd` leaves them out.
    pub unpriced_models: BTreeSet<String>,
}

impl SessionSummary {
    fn empty(id: &str, project_key: &str) -> Self {
        SessionSummary {
            id: id.to_string(),
            id_known: true,
            project_key: project_key.to_string(),
            project_path: None,
            file_path: None,
            source: Source::Transcript,
            transcript_missing: false,
            favorite: false,
            title: None,
            title_rank: 0,
            first_prompt: String::new(),
            last_prompt: None,
            last_message: None,
            started_at: None,
            updated_at: 0,
            git_branch: None,
            cwd: None,
            cc_version: None,
            entrypoint: None,
            permission_mode: None,
            is_background: false,
            user_msg_count: 0,
            assistant_msg_count: 0,
            tool_use_count: 0,
            tool_counts: BTreeMap::new(),
            subagent_count: 0,
            file_size: None,
            total_cost_usd: None,
            duration_ms: None,
            lines_added: None,
            lines_removed: None,
            pr_url: None,
            files_touched: BTreeSet::new(),
            models: BTreeSet::new(),
            cost_usd: None,
            tokens_in: 0,
            tokens_out: 0,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
            context_tokens: None,
            context_peak: None,
            context_model: None,
            unpriced_models: BTreeSet::new(),
        }
    }

    /// A fresh row for a transcript file, before any record is applied.
    pub fn transcript(id: &str, project_key: &str, path: &Path) -> Self {
        let mut s = Self::empty(id, project_key);
        s.file_path = Some(path.to_path_buf());
        s
    }

    /// A fresh row recovered from `history.jsonl`.
    pub fn history(id: &str, id_known: bool, project_key: &str, project_path: &str) -> Self {
        let mut s = Self::empty(id, project_key);
        s.id_known = id_known;
        s.source = Source::History;
        s.project_path = Some(project_path.to_string());
        s
    }

    /// Title to display: explicit title, else the first line of the first prompt.
    pub fn display_title(&self) -> &str {
        match &self.title {
            Some(t) if !t.is_empty() => t,
            _ => self.first_prompt.lines().next().unwrap_or(""),
        }
    }

    /// True when the row should be shown as "history only" (no transcript on disk).
    pub fn history_only(&self) -> bool {
        self.source == Source::History || self.transcript_missing
    }

    /// How full the context window was at the end: last main-thread prompt / the model's window.
    pub fn context_ratio(&self) -> Option<f64> {
        let window = crate::pricing::context_window(self.context_model.as_deref()?)?;
        Some(self.context_tokens? as f64 / window as f64)
    }

    /// Set the cost and token totals from a session's API responses. Rows sharing a `msg_id`
    /// count once (the last one wins), like the index's `api_usage` table.
    pub fn set_usage(&mut self, usage: &[UsageRow]) {
        let mut by_id: BTreeMap<&str, &UsageRow> = BTreeMap::new();
        for u in usage {
            by_id.insert(&u.msg_id, u);
        }
        self.cost_usd = None;
        (self.tokens_in, self.tokens_out, self.tokens_cache_read, self.tokens_cache_write) = (0, 0, 0, 0);
        self.unpriced_models.clear();
        for u in by_id.values() {
            self.tokens_in += u.input;
            self.tokens_out += u.output;
            self.tokens_cache_read += u.cache_read;
            self.tokens_cache_write += u.cache_write_5m + u.cache_write_1h;
            match u.cost_usd {
                Some(c) => *self.cost_usd.get_or_insert(0.0) += c,
                None => drop(self.unpriced_models.insert(u.model.clone())),
            }
        }
    }
}

/// One API response's token usage. Claude Code writes a response as one record per content
/// block, all carrying the same `message.id` and the same usage: they collapse into one row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageRow {
    /// `message.id`, else `requestId`, else the record's uuid.
    pub msg_id: String,
    pub model: String,
    /// Unix ms; 0 when the record has no timestamp.
    pub ts: i64,
    /// Uncached input tokens.
    pub input: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
    pub output: u64,
    pub web_search: u64,
    /// `usage.speed == "fast"`.
    pub fast: bool,
    /// `pricing::cost` of this response; `None` for a model without a known price.
    pub cost_usd: Option<f64>,
}

impl UsageRow {
    /// Every input token the request sent, cached or not: how full the context was.
    pub fn prompt_tokens(&self) -> u64 {
        self.input + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }
}

/// One searchable message: the text blocks of a user or assistant record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageRow {
    pub uuid: String,
    pub parent_uuid: Option<String>,
    pub role: Role,
    /// Unix ms; 0 when the record has no timestamp.
    pub ts: i64,
    pub text: String,
    /// Harness-injected user text (`isMeta`), excluded from full-text search.
    pub is_meta: bool,
    pub byte_offset: u64,
    pub byte_len: u64,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

/// A compact transcript handoff and diagnostics for its caller (CLI or GUI).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptExport {
    pub text: String,
    /// Malformed JSON records, including a partially written final line.
    pub skipped_lines: usize,
}

/// A directly read transcript, in file order, independent of the SQLite index.
#[derive(Debug)]
pub struct Conversation {
    pub items: Vec<ConvItem>,
    /// File size at read time, including any unfinished tail.
    pub bytes: u64,
    /// Invalid complete JSON lines; an unfinished final line is not counted.
    pub skipped_lines: usize,
}
#[derive(Debug)]
pub enum ConvItem { Turn(Turn), Compact(Compaction) }
#[derive(Debug)]
pub struct Turn {
    /// Human turns count from one; automatic segments have no number.
    pub number: Option<u32>,
    pub origin: Option<String>,
    pub ts: Option<i64>,
    pub prompt: Marked,
    pub attachments: Vec<Attachment>,
    pub steps: Vec<Step>,
    pub reply: Option<Reply>,
    pub end: TurnEnd,
    pub duration_ms: Option<i64>,
    pub last_ts: Option<i64>,
}
#[derive(Debug)]
pub struct Attachment { pub kind: String, pub bytes: u64 }
#[derive(Debug)]
pub struct Step { pub ts: Option<i64>, pub kind: StepKind }
#[derive(Debug)]
pub enum StepKind { Text(Marked), Thinking(Marked), Tool(ToolCall) }
#[derive(Debug)]
pub struct ToolCall {
    pub name: String,
    /// Single-line argument preview, at most 300 characters; paths remain absolute.
    pub arg: String,
    pub input: Vec<crate::search::regex::ToolLine>,
    pub result: Option<ToolResult>,
}
#[derive(Debug)]
pub struct ToolResult { pub output: Marked, pub bytes: usize, pub lines: usize, pub is_error: bool }
#[derive(Debug)]
pub struct Reply { pub ts: Option<i64>, pub text: Marked }
#[derive(Debug)]
pub enum TurnEnd { Completed, Interrupted { ts: Option<i64> }, ApiError { ts: Option<i64>, text: String }, NoReply }
#[derive(Debug)]
pub struct Compaction { pub ts: Option<i64>, pub summary: Option<Marked> }
#[derive(Debug, Default)]
pub struct Marked {
    pub text: String,
    /// UTF-8 byte ranges in `text`, after truncation and omission markers.
    pub hits: Vec<std::ops::Range<usize>>,
    /// Original character and source-line counts, before truncation.
    pub chars: usize,
    pub lines: usize,
    /// Characters of the original that `text` holds, omission marks not counted; less than
    /// `chars` when the text was cut.
    pub kept: usize,
    /// The retained window moved to the first match and begins with an ellipsis.
    pub moved: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part { Prompt, Step(usize), Reply, Summary }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitPlace { pub item: usize, pub part: Part }
