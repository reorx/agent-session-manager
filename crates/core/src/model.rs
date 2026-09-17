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
