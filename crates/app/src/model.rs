//! View-model types for the UI. Everything the table renders is pre-formatted into
//! `SharedString`s so `render_td` never allocates.

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_session_core::format::{fmt_datetime, fmt_time};
use agent_session_core::model::{MessageRow, Role, SessionSummary};
use agent_session_core::search::Matcher;
use chrono::{Days, Local, TimeZone};
use gpui_kit::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Cli,
    Sdk,
    Background,
    History,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Cli, Kind::Sdk, Kind::Background, Kind::History];

    /// Short name for the Kind filter (button value and menu rows).
    pub fn short(self) -> &'static str {
        match self {
            Kind::Cli => "CLI",
            Kind::Sdk => "SDK",
            Kind::Background => "BG",
            Kind::History => "History",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Cli => "CLI",
            Kind::Sdk => "SDK (Agent SDK)",
            Kind::Background => "BG (background)",
            Kind::History => "History (record cleaned up)",
        }
    }
}

/// Preview strings are flattened to one line and cut here (the reference script's `PROMPT_LINE_LEN`).
const PREVIEW_CHARS: usize = 240;
/// Multi-line texts shown in the Metadata tab are cut here.
const DETAIL_CHARS: usize = 2000;
const NONE: &str = "—";

/// One session row, built once from a core `SessionSummary`. Display strings are pre-formatted
/// so `render_td` does no work.
pub struct SessionRow {
    pub id: SharedString,
    pub short_id: SharedString,
    pub project_key: SharedString,
    /// Last segment of the project path.
    pub project: SharedString,
    pub project_path: SharedString,
    /// The transcript a regex search scans; `None` once Claude Code cleaned it up.
    pub transcript_path: Option<PathBuf>,
    pub title: SharedString,
    pub first_prompt: SharedString,
    pub last_prompt: SharedString,
    pub last_message: SharedString,
    /// `i64::MIN` when the session never got a prompt, so it sorts before everything.
    pub started_at: i64,
    pub updated_at: i64,
    pub started_fmt: SharedString,
    pub updated_fmt: SharedString,
    pub started_full: SharedString,
    pub updated_full: SharedString,
    pub prompts: u32,
    pub prompts_fmt: SharedString,
    pub size: u64,
    pub size_fmt: SharedString,
    pub cost_fmt: SharedString,
    pub branch: SharedString,
    pub tools: u32,
    pub tools_fmt: SharedString,
    pub kind: Kind,
    // Metadata tab only.
    pub messages_fmt: SharedString,
    pub duration_fmt: SharedString,
    pub models: SharedString,
    pub permission_mode: SharedString,
    pub version: SharedString,
    pub top_tools: SharedString,
    pub subagents_fmt: SharedString,
    pub pr_url: SharedString,
    pub resume: SharedString,
    pub first_prompt_full: SharedString,
    pub last_prompt_full: SharedString,
    pub last_message_full: SharedString,
}

fn cut(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((byte_ix, _)) => format!("{}…", &text[..byte_ix]),
        None => text.to_string(),
    }
}

/// Flatten to one line (`⏎` marks line breaks) and cut to the preview length.
fn single_line(text: &str) -> SharedString {
    let flat = text.trim().replace("\r\n", "⏎").replace('\n', "⏎");
    if flat.is_empty() { NONE.into() } else { cut(&flat, PREVIEW_CHARS).into() }
}

fn detail_text(text: Option<&str>) -> SharedString {
    match text.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => cut(t, DETAIL_CHARS).into(),
        None => NONE.into(),
    }
}

fn or_none(text: Option<&str>) -> SharedString {
    text.filter(|t| !t.is_empty()).unwrap_or(NONE).to_string().into()
}

fn format_size(bytes: Option<u64>) -> SharedString {
    let Some(bytes) = bytes else { return NONE.into() };
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 { format!("{kb:.0} KB").into() } else { format!("{:.1} MB", kb / 1024.0).into() }
}

fn format_duration(ms: Option<i64>) -> SharedString {
    let Some(ms) = ms.filter(|ms| *ms > 0) else { return NONE.into() };
    let minutes = ms / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, 0) => format!("{}s", ms / 1000).into(),
        (0, m) => format!("{m}m").into(),
        (h, m) => format!("{h}h {m}m").into(),
    }
}

fn project_name(s: &SessionSummary) -> String {
    let path = s.project_path.as_deref().or(s.cwd.as_deref());
    match path.and_then(|p| Path::new(p).file_name()) {
        Some(name) => name.to_string_lossy().into_owned(),
        None => s.project_key.clone(),
    }
}

fn top_tools(s: &SessionSummary) -> SharedString {
    let mut tools: Vec<(&String, &u32)> = s.tool_counts.iter().collect();
    tools.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let top: Vec<String> = tools.iter().take(5).map(|(name, n)| format!("{name} {n}")).collect();
    if top.is_empty() { NONE.into() } else { top.join(" · ").into() }
}

impl Kind {
    pub fn of(s: &SessionSummary) -> Kind {
        if s.history_only() {
            Kind::History
        } else if s.is_background {
            Kind::Background
        } else if s.entrypoint.as_deref() == Some("sdk-cli") {
            Kind::Sdk
        } else {
            Kind::Cli
        }
    }
}

impl SessionRow {
    pub fn from_summary(s: &SessionSummary) -> SessionRow {
        let short_id = if s.id_known { s.id.chars().take(8).collect() } else { "(unknown)".to_string() };
        let resume = if s.id_known { format!("claude --resume {}", s.id) } else { NONE.to_string() };
        SessionRow {
            id: s.id.clone().into(),
            short_id: short_id.into(),
            project_key: s.project_key.clone().into(),
            project: project_name(s).into(),
            project_path: or_none(s.project_path.as_deref().or(s.cwd.as_deref())),
            transcript_path: s.file_path.clone().filter(|_| !s.history_only()),
            title: single_line(s.display_title()),
            first_prompt: single_line(&s.first_prompt),
            last_prompt: single_line(s.last_prompt.as_deref().unwrap_or("")),
            last_message: single_line(s.last_message.as_deref().unwrap_or("")),
            started_at: s.started_at.unwrap_or(i64::MIN),
            updated_at: s.updated_at,
            started_fmt: s.started_at.map_or(NONE.into(), |ms| fmt_time(Some(ms)).into()),
            updated_fmt: fmt_time(Some(s.updated_at)).into(),
            started_full: s.started_at.map_or(NONE.into(), |ms| fmt_datetime(Some(ms)).into()),
            updated_full: fmt_datetime(Some(s.updated_at)).into(),
            prompts: s.user_msg_count,
            prompts_fmt: s.user_msg_count.to_string().into(),
            size: s.file_size.unwrap_or(0),
            size_fmt: format_size(s.file_size),
            cost_fmt: s.total_cost_usd.map_or(NONE.into(), |c| format!("${c:.2}").into()),
            branch: or_none(s.git_branch.as_deref()),
            tools: s.tool_use_count,
            tools_fmt: s.tool_use_count.to_string().into(),
            kind: Kind::of(s),
            messages_fmt: (s.user_msg_count + s.assistant_msg_count).to_string().into(),
            duration_fmt: format_duration(s.duration_ms),
            models: if s.models.is_empty() { NONE.into() } else { s.models.iter().cloned().collect::<Vec<_>>().join(", ").into() },
            permission_mode: or_none(s.permission_mode.as_deref()),
            version: or_none(s.cc_version.as_deref()),
            top_tools: top_tools(s),
            subagents_fmt: s.subagent_count.to_string().into(),
            pr_url: or_none(s.pr_url.as_deref()),
            resume: resume.into(),
            first_prompt_full: detail_text(Some(&s.first_prompt)),
            last_prompt_full: detail_text(s.last_prompt.as_deref()),
            last_message_full: detail_text(s.last_message.as_deref()),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum DisplayMode {
    Title,
    #[default]
    FirstPrompt,
    LastPrompt,
    LastMessage,
}

impl DisplayMode {
    pub const ALL: [DisplayMode; 4] = [
        DisplayMode::Title,
        DisplayMode::FirstPrompt,
        DisplayMode::LastPrompt,
        DisplayMode::LastMessage,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DisplayMode::Title => "Title",
            DisplayMode::FirstPrompt => "First prompt",
            DisplayMode::LastPrompt => "Last prompt",
            DisplayMode::LastMessage => "Last message",
        }
    }

    pub fn shortcut(self) -> &'static str {
        match self {
            DisplayMode::Title => "⌘1",
            DisplayMode::FirstPrompt => "⌘2",
            DisplayMode::LastPrompt => "⌘3",
            DisplayMode::LastMessage => "⌘4",
        }
    }

    pub fn preview(self, row: &SessionRow) -> SharedString {
        match self {
            DisplayMode::Title => row.title.clone(),
            DisplayMode::FirstPrompt => row.first_prompt.clone(),
            DisplayMode::LastPrompt => row.last_prompt.clone(),
            DisplayMode::LastMessage => row.last_message.clone(),
        }
    }
}

/// Search scope: which message roles a query runs against.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ScopeSet {
    pub user: bool,
    pub assistant: bool,
    pub tools: bool,
}

impl Default for ScopeSet {
    fn default() -> Self {
        Self { user: true, assistant: true, tools: false }
    }
}

impl ScopeSet {
    pub const ITEMS: [(&'static str, &'static str); 3] =
        [("User", "prompts"), ("Assistant", "replies"), ("Tools", "regex only")];

    pub fn get(&self, ix: usize) -> bool {
        match ix {
            0 => self.user,
            1 => self.assistant,
            _ => self.tools,
        }
    }

    pub fn toggle(&mut self, ix: usize) {
        match ix {
            0 => self.user = !self.user,
            1 => self.assistant = !self.assistant,
            _ => self.tools = !self.tools,
        }
    }

    pub fn label(&self) -> String {
        match (self.user, self.assistant, self.tools) {
            (true, true, true) => "All".into(),
            (true, true, false) => "Both".into(),
            (false, false, false) => "None".into(),
            _ => {
                let mut on = Vec::new();
                if self.user {
                    on.push("User");
                }
                if self.assistant {
                    on.push("Assistant");
                }
                if self.tools {
                    on.push("Tools");
                }
                on.join(", ")
            }
        }
    }
}

/// Which session kinds the table shows. Every kind is on by default, and a kind a saved file
/// does not mention stays on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct KindSet {
    pub cli: bool,
    pub sdk: bool,
    pub bg: bool,
    pub history: bool,
}

impl Default for KindSet {
    fn default() -> Self {
        Self { cli: true, sdk: true, bg: true, history: true }
    }
}

impl KindSet {
    pub fn contains(&self, kind: Kind) -> bool {
        match kind {
            Kind::Cli => self.cli,
            Kind::Sdk => self.sdk,
            Kind::Background => self.bg,
            Kind::History => self.history,
        }
    }

    pub fn toggle(&mut self, kind: Kind) {
        match kind {
            Kind::Cli => self.cli = !self.cli,
            Kind::Sdk => self.sdk = !self.sdk,
            Kind::Background => self.bg = !self.bg,
            Kind::History => self.history = !self.history,
        }
    }

    pub fn label(&self) -> String {
        let on: Vec<&str> = Kind::ALL.iter().filter(|k| self.contains(**k)).map(|k| k.short()).collect();
        match on.len() {
            0 => "None".into(),
            n if n == Kind::ALL.len() => "All".into(),
            _ => on.join(", "),
        }
    }
}

/// Sessions per kind, indexed by `Kind as usize`.
pub fn kind_counts<'a>(rows: impl Iterator<Item = &'a Arc<SessionRow>>) -> [usize; Kind::ALL.len()] {
    let mut counts = [0; Kind::ALL.len()];
    for row in rows {
        counts[row.kind as usize] += 1;
    }
    counts
}

/// Updated-time filter presets.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum RangePreset {
    Today,
    Days3,
    Days7,
    Days30,
    #[default]
    Any,
}

impl RangePreset {
    pub const ALL: [RangePreset; 5] =
        [RangePreset::Today, RangePreset::Days3, RangePreset::Days7, RangePreset::Days30, RangePreset::Any];

    pub fn label(self) -> &'static str {
        match self {
            RangePreset::Today => "Today",
            RangePreset::Days3 => "3 days",
            RangePreset::Days7 => "7 days",
            RangePreset::Days30 => "30 days",
            RangePreset::Any => "Any time",
        }
    }

    /// Lower bound (ms since epoch) for `updated_at`, or `None` for no filter.
    pub fn since_ms(self) -> Option<i64> {
        let days = match self {
            RangePreset::Today => 0,
            RangePreset::Days3 => 2,
            RangePreset::Days7 => 6,
            RangePreset::Days30 => 29,
            RangePreset::Any => return None,
        };
        let today = Local::now().date_naive();
        let start = today.checked_sub_days(Days::new(days)).unwrap_or(today);
        let start = start.and_hms_opt(0, 0, 0)?.and_local_timezone(Local).single()?;
        Some(start.timestamp_millis())
    }
}

/// Filter applied to the table rows before sorting.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Filter {
    /// `project_key` of the only project to show.
    pub project: Option<SharedString>,
    pub since_ms: Option<i64>,
    /// Sessions that never got a human prompt are hidden unless this is set.
    pub show_empty: bool,
    pub kinds: KindSet,
    /// `project_key`s whose sessions never show (noise such as a probe tool's directory).
    pub hidden_projects: BTreeSet<SharedString>,
}

impl Filter {
    pub fn matches(&self, row: &SessionRow) -> bool {
        let project_ok = self.project.as_ref().is_none_or(|p| row.project_key == *p);
        let since_ok = self.since_ms.is_none_or(|since| row.updated_at >= since);
        let shown = (self.show_empty || row.prompts > 0) && !self.hidden_projects.contains(&row.project_key);
        project_ok && since_ok && shown && self.kinds.contains(row.kind)
    }

    /// Hide a project; when it is the picked one the filter goes back to all projects.
    pub fn hide_project(&mut self, key: SharedString) {
        if self.project.as_ref() == Some(&key) {
            self.project = None;
        }
        self.hidden_projects.insert(key);
    }

    pub fn show_project(&mut self, key: &str) {
        self.hidden_projects.remove(key);
    }
}

/// One entry of the Project dropdown.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectEntry {
    pub key: SharedString,
    pub name: SharedString,
    pub count: usize,
    pub last_updated: i64,
}

/// Distinct projects of `rows`, most recently active first.
pub fn project_entries<'a>(rows: impl Iterator<Item = &'a Arc<SessionRow>>) -> Vec<ProjectEntry> {
    let mut by_key: HashMap<SharedString, ProjectEntry> = HashMap::new();
    for row in rows {
        let entry = by_key.entry(row.project_key.clone()).or_insert_with(|| ProjectEntry {
            key: row.project_key.clone(),
            name: row.project.clone(),
            count: 0,
            last_updated: i64::MIN,
        });
        entry.count += 1;
        entry.last_updated = entry.last_updated.max(row.updated_at);
        if entry.name == entry.key {
            // a row without cwd only knows the encoded key; any sibling with a real path names the project
            entry.name = row.project.clone();
        }
    }
    let mut out: Vec<ProjectEntry> = by_key.into_values().collect();
    out.sort_by(|a, b| b.last_updated.cmp(&a.last_updated).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Split the project list into the projects the dropdown offers and the hidden ones. A hidden
/// key without any session left is still listed (by its key), so it can be shown again.
pub fn split_hidden(entries: Vec<ProjectEntry>, hidden: &BTreeSet<SharedString>) -> (Vec<ProjectEntry>, Vec<ProjectEntry>) {
    let (mut hidden_entries, listed): (Vec<_>, Vec<_>) = entries.into_iter().partition(|e| hidden.contains(&e.key));
    for key in hidden {
        if !hidden_entries.iter().any(|e| e.key == *key) {
            hidden_entries.push(ProjectEntry { key: key.clone(), name: key.clone(), count: 0, last_updated: i64::MIN });
        }
    }
    (listed, hidden_entries)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MessageKind {
    User,
    Assistant,
    // Rendered by the detail panel already; built once transcripts are read directly (M4).
    #[allow(dead_code)]
    Thinking,
    #[allow(dead_code)]
    Tool,
}

/// One entry of a session transcript, pre-formatted for the conversation view.
pub struct Message {
    pub kind: MessageKind,
    pub time: SharedString,
    pub text: SharedString,
    /// Search hits inside `text`.
    pub highlights: Vec<Range<usize>>,
    /// Tool name (tool messages only).
    pub name: SharedString,
    /// Tool argument summary (tool messages only).
    pub arg: SharedString,
    /// Tool result summary, e.g. `212 lines` (tool messages only).
    pub result: SharedString,
    pub input: SharedString,
    pub output: SharedString,
    pub more: SharedString,
    /// Thinking length summary, e.g. `1.2k chars`.
    pub meta: SharedString,
    /// First 60 chars of thinking, shown while collapsed.
    pub preview: SharedString,
}

pub type Conversation = Vec<Arc<Message>>;

/// Messages rendered at once; the conversation view is not virtualized until M4.
pub const CONVERSATION_CAP: usize = 300;
const MESSAGE_CHARS: usize = 4000;
/// Messages shown ahead of the first search hit when the window has to move to reach it.
const WINDOW_LEAD: usize = 5;
/// Characters shown ahead of the first hit of a message whose hits are all past `MESSAGE_CHARS`.
const HIT_LEAD_CHARS: usize = 200;

/// The slice of a session the conversation view renders.
#[derive(Default)]
pub struct ConversationView {
    pub messages: Conversation,
    /// Messages left out ahead of the first one: the window moves when the first search hit
    /// lies beyond `CONVERSATION_CAP`.
    pub skipped: usize,
    /// Prompts among the skipped messages, so `USER #n` keeps counting from the real start.
    pub skipped_turns: usize,
    /// Messages the session has in the index, shown or not.
    pub total: usize,
}

/// The text a message shows and the search hits in it. A hit past the cut moves the cut.
fn shown_text(text: &str, matcher: Option<&Matcher>) -> (String, Vec<Range<usize>>) {
    let text = text.trim();
    let Some(matcher) = matcher else { return (cut(text, MESSAGE_CHARS), Vec::new()) };
    let limit = text.char_indices().nth(MESSAGE_CHARS).map_or(text.len(), |(i, _)| i);
    let shown = match matcher.ranges(text).first() {
        Some(first) if first.end > limit => {
            let lead = text[..first.start].char_indices().rev().nth(HIT_LEAD_CHARS - 1).map_or(0, |(i, _)| i);
            format!("…{}", cut(&text[lead..], MESSAGE_CHARS))
        }
        _ => cut(text, MESSAGE_CHARS),
    };
    let ranges = matcher.ranges(&shown);
    (shown, ranges)
}

impl Message {
    fn text(kind: MessageKind, ts: i64, text: &str, matcher: Option<&Matcher>) -> Message {
        let time = Local.timestamp_millis_opt(ts).single().filter(|_| ts > 0).map(|dt| dt.format("%H:%M").to_string());
        let (text, highlights) = shown_text(text, matcher);
        let empty = SharedString::default;
        Message {
            kind,
            time: time.unwrap_or_default().into(),
            text: text.into(),
            highlights,
            name: empty(),
            arg: empty(),
            result: empty(),
            input: empty(),
            output: empty(),
            more: empty(),
            meta: empty(),
            preview: empty(),
        }
    }
}

/// The indexed user / assistant texts of a session (no tool calls or thinking: those are read
/// from the transcript in M4). Harness-injected user messages are skipped. With a `matcher`
/// the hits are marked, and the window starts shortly before the first one if it would
/// otherwise fall outside.
pub fn conversation_from(messages: &[MessageRow], matcher: Option<&Matcher>) -> ConversationView {
    let shown: Vec<&MessageRow> = messages.iter().filter(|m| !m.is_meta && !m.text.trim().is_empty()).collect();
    let first_hit = matcher.and_then(|matcher| shown.iter().position(|m| matcher.is_match(m.text.trim())));
    let skipped = first_hit.filter(|ix| *ix >= CONVERSATION_CAP).map_or(0, |ix| ix - WINDOW_LEAD);
    let messages = shown[skipped..]
        .iter()
        .take(CONVERSATION_CAP)
        .map(|m| {
            let kind = if m.role == Role::User { MessageKind::User } else { MessageKind::Assistant };
            Arc::new(Message::text(kind, m.ts, &m.text, matcher))
        })
        .collect();
    let skipped_turns = shown[..skipped].iter().filter(|m| m.role == Role::User).count();
    ConversationView { messages, skipped, skipped_turns, total: shown.len() }
}

#[cfg(test)]
pub mod tests {
    // `gpui_kit::*` exports gpui's own `test` attribute; name the standard one explicitly.
    use core::prelude::v1::test;

    use super::*;

    pub fn summary(id: &str, project_path: &str, updated_at: i64) -> SessionSummary {
        let key = agent_session_core::claude::paths::encode_project_path(project_path);
        let mut s = SessionSummary::transcript(id, &key, Path::new("/tmp/x.jsonl"));
        s.project_path = Some(project_path.to_string());
        s.first_prompt = format!("prompt of {id}");
        s.user_msg_count = 1;
        s.started_at = Some(updated_at - 1000);
        s.updated_at = updated_at;
        s
    }

    pub fn row(id: &str, project_path: &str, updated_at: i64) -> Arc<SessionRow> {
        Arc::new(SessionRow::from_summary(&summary(id, project_path, updated_at)))
    }

    #[test]
    fn a_row_shows_the_project_folder_name_and_a_short_id() {
        let r = SessionRow::from_summary(&summary("11111111-aaaa-4bbb-8ccc-000000000001", "/Users/me/Code/alpha", 5000));
        assert_eq!(r.project, "alpha");
        assert_eq!(r.project_path, "/Users/me/Code/alpha");
        assert_eq!(r.project_key, "-Users-me-Code-alpha");
        assert_eq!(r.short_id, "11111111");
        assert_eq!(r.resume, "claude --resume 11111111-aaaa-4bbb-8ccc-000000000001");
    }

    #[test]
    fn a_row_without_any_path_falls_back_to_the_encoded_key() {
        let mut s = summary("s", "/Users/me/Code/alpha", 5000);
        s.project_path = None;
        assert_eq!(SessionRow::from_summary(&s).project, "-Users-me-Code-alpha");
        s.cwd = Some("/Users/me/Code/alpha/".into());
        assert_eq!(SessionRow::from_summary(&s).project, "alpha", "cwd is as good as the project path");
    }

    #[test]
    fn previews_are_one_line_and_capped_while_the_detail_text_keeps_line_breaks() {
        let mut s = summary("s", "/p/a", 5000);
        s.first_prompt = format!("line one\nline two\r\n{}", "x".repeat(400));
        s.last_message = Some("  done  \n".into());
        let r = SessionRow::from_summary(&s);
        assert!(r.first_prompt.starts_with("line one⏎line two⏎xxx"));
        assert_eq!(r.first_prompt.chars().count(), 241, "240 chars + ellipsis");
        assert!(r.first_prompt.ends_with('…'));
        assert!(r.first_prompt_full.contains("line one\nline two"));
        assert_eq!(r.title, "line one", "without an explicit title the first prompt line is the title");
        assert_eq!(r.last_message, "done");
        assert_eq!(r.last_prompt, "—", "missing texts render as a dash");
    }

    #[test]
    fn an_explicit_title_wins_over_the_first_prompt() {
        let mut s = summary("s", "/p/a", 5000);
        s.title = Some("Refactor the parser".into());
        assert_eq!(SessionRow::from_summary(&s).title, "Refactor the parser");
    }

    #[test]
    fn kind_is_history_then_background_then_sdk_then_cli() {
        let mut s = summary("s", "/p/a", 5000);
        assert_eq!(Kind::of(&s), Kind::Cli);
        s.entrypoint = Some("sdk-cli".into());
        assert_eq!(Kind::of(&s), Kind::Sdk);
        s.is_background = true;
        assert_eq!(Kind::of(&s), Kind::Background);
        s.transcript_missing = true;
        assert_eq!(Kind::of(&s), Kind::History);
        assert_eq!(Kind::of(&SessionSummary::history("h", true, "-p-a", "/p/a")), Kind::History);
    }

    #[test]
    fn missing_numbers_render_as_dashes_and_sort_low() {
        let s = SessionSummary::history("history:-p-a:1", false, "-p-a", "/p/a");
        let r = SessionRow::from_summary(&s);
        assert_eq!((r.size_fmt.as_ref(), r.size), ("—", 0));
        assert_eq!((r.started_fmt.as_ref(), r.started_at), ("—", i64::MIN));
        assert_eq!(r.cost_fmt, "—");
        assert_eq!(r.short_id, "(unknown)");
        assert_eq!(r.resume, "—", "an anonymous history session cannot be resumed");
    }

    #[test]
    fn sizes_durations_and_tools_are_human_readable() {
        let mut s = summary("s", "/p/a", 5000);
        s.file_size = Some(3 * 1024 * 1024 + 512 * 1024);
        s.duration_ms = Some(3_900_000);
        s.total_cost_usd = Some(1.966);
        s.tool_counts = [("Bash".to_string(), 7), ("Read".to_string(), 12), ("Edit".to_string(), 7)].into();
        let r = SessionRow::from_summary(&s);
        assert_eq!(r.size_fmt, "3.5 MB");
        assert_eq!(r.duration_fmt, "1h 5m");
        assert_eq!(r.cost_fmt, "$1.97");
        assert_eq!(r.top_tools, "Read 12 · Bash 7 · Edit 7");
        s.file_size = Some(2048);
        assert_eq!(SessionRow::from_summary(&s).size_fmt, "2 KB");
    }

    #[test]
    fn the_filter_matches_project_key_time_and_hides_empty_sessions() {
        let a = row("a", "/p/alpha", 10_000);
        let b = row("b", "/p/beta", 20_000);
        let mut empty = summary("e", "/p/alpha", 30_000);
        empty.user_msg_count = 0;
        let empty = SessionRow::from_summary(&empty);

        let all = Filter::default();
        assert!(all.matches(&a) && all.matches(&b));
        assert!(!all.matches(&empty), "sessions without a prompt are hidden by default");
        assert!(Filter { show_empty: true, ..Default::default() }.matches(&empty));

        let alpha = Filter { project: Some("-p-alpha".into()), ..Default::default() };
        assert!(alpha.matches(&a) && !alpha.matches(&b));
        let recent = Filter { since_ms: Some(15_000), ..Default::default() };
        assert!(!recent.matches(&a) && recent.matches(&b));
    }

    fn no_kinds() -> KindSet {
        KindSet { cli: false, sdk: false, bg: false, history: false }
    }

    fn kind_row(id: &str, kind: Kind) -> Arc<SessionRow> {
        let mut s = summary(id, "/p/alpha", 10_000);
        match kind {
            Kind::Cli => {}
            Kind::Sdk => s.entrypoint = Some("sdk-cli".into()),
            Kind::Background => s.is_background = true,
            Kind::History => s.transcript_missing = true,
        }
        let row = SessionRow::from_summary(&s);
        assert_eq!(row.kind, kind);
        Arc::new(row)
    }

    #[test]
    fn the_filter_shows_every_kind_by_default_and_only_the_picked_ones_after_that() {
        let rows: Vec<Arc<SessionRow>> = Kind::ALL.iter().map(|k| kind_row(k.short(), *k)).collect();
        let all = Filter::default();
        assert!(rows.iter().all(|r| all.matches(r)), "no source is hidden by default");

        let mut kinds = KindSet::default();
        kinds.toggle(Kind::History);
        let no_history = Filter { kinds, ..Default::default() };
        let shown: Vec<&str> = rows.iter().filter(|r| no_history.matches(r)).map(|r| r.id.as_ref()).collect();
        assert_eq!(shown, ["CLI", "SDK", "BG"]);

        let none = Filter { kinds: no_kinds(), ..Default::default() };
        assert!(!rows.iter().any(|r| none.matches(r)), "nothing picked, nothing shown");
    }

    #[test]
    fn the_kind_filter_stacks_with_project_time_and_empty_sessions() {
        let mut kinds = no_kinds();
        kinds.toggle(Kind::Cli);
        let cli_alpha = Filter { kinds, project: Some("-p-alpha".into()), since_ms: Some(5_000), ..Default::default() };
        assert!(cli_alpha.matches(&kind_row("a", Kind::Cli)));
        assert!(!cli_alpha.matches(&kind_row("s", Kind::Sdk)), "wrong kind");
        assert!(!cli_alpha.matches(&row("b", "/p/beta", 10_000)), "right kind, wrong project");
        assert!(!cli_alpha.matches(&row("old", "/p/alpha", 1_000)), "right kind, too old");
        let mut empty = summary("e", "/p/alpha", 10_000);
        empty.user_msg_count = 0;
        assert!(!cli_alpha.matches(&SessionRow::from_summary(&empty)), "right kind, no prompt");
    }

    #[test]
    fn the_kind_button_reads_all_none_or_the_short_names() {
        let mut kinds = KindSet::default();
        assert_eq!(kinds.label(), "All");
        kinds.toggle(Kind::Background);
        kinds.toggle(Kind::History);
        assert_eq!(kinds.label(), "CLI, SDK");
        assert!(kinds.contains(Kind::Sdk) && !kinds.contains(Kind::History));
        kinds.toggle(Kind::History);
        assert_eq!(kinds.label(), "CLI, SDK, History");
        assert_eq!(no_kinds().label(), "None");
    }

    #[test]
    fn kinds_are_counted_per_kind() {
        let rows = vec![kind_row("a", Kind::Cli), kind_row("b", Kind::History), kind_row("c", Kind::History), kind_row("d", Kind::Sdk)];
        let counts = kind_counts(rows.iter());
        let got: Vec<(&str, usize)> = Kind::ALL.iter().map(|k| (k.short(), counts[*k as usize])).collect();
        assert_eq!(got, [("CLI", 1), ("SDK", 1), ("BG", 0), ("History", 2)]);
    }

    #[test]
    fn rows_of_a_hidden_project_are_filtered_out_whatever_else_matches() {
        let (a, b) = (row("a", "/p/alpha", 10_000), row("b", "/p/beta", 20_000));
        let mut filter = Filter::default();
        filter.hide_project("-p-beta".into());
        assert!(filter.matches(&a) && !filter.matches(&b));
        assert!(!Filter { show_empty: true, since_ms: Some(1), ..filter.clone() }.matches(&b), "still hidden");
        filter.show_project("-p-beta");
        assert_eq!(filter, Filter::default(), "showing it again leaves no trace");
    }

    #[test]
    fn hiding_the_picked_project_goes_back_to_all_projects() {
        let mut filter = Filter { project: Some("-p-beta".into()), ..Default::default() };
        filter.hide_project("-p-alpha".into());
        assert_eq!(filter.project.as_deref(), Some("-p-beta"), "another project stays picked");
        filter.hide_project("-p-beta".into());
        assert_eq!(filter.project, None, "otherwise the table would be stuck on an empty project");
        assert!(filter.matches(&row("g", "/p/gamma", 1)));
    }

    #[test]
    fn hidden_projects_leave_the_project_list_and_are_listed_apart() {
        let rows = vec![row("a", "/p/alpha", 10_000), row("b", "/p/beta", 20_000), row("c", "/p/alpha", 30_000)];
        let hidden: BTreeSet<SharedString> = ["-p-alpha".into(), "-p-gone".into()].into();
        let (listed, hidden) = split_hidden(project_entries(rows.iter()), &hidden);
        let names = |entries: &[ProjectEntry]| -> Vec<(String, usize)> { entries.iter().map(|e| (e.name.to_string(), e.count)).collect() };
        assert_eq!(names(&listed), [("beta".to_string(), 1)]);
        assert_eq!(
            names(&hidden),
            [("alpha".to_string(), 2), ("-p-gone".to_string(), 0)],
            "a hidden project without any session left is still listed, so it can be shown again"
        );
    }

    #[test]
    fn projects_are_listed_most_recently_active_first_with_counts() {
        let rows = vec![row("a", "/p/alpha", 10_000), row("b", "/p/beta", 20_000), row("c", "/p/alpha", 30_000)];
        let entries = project_entries(rows.iter());
        let got: Vec<(&str, usize, i64)> = entries.iter().map(|e| (e.name.as_ref(), e.count, e.last_updated)).collect();
        assert_eq!(got, [("alpha", 2, 30_000), ("beta", 1, 20_000)]);
        assert_eq!(entries[0].key, "-p-alpha");
    }

    #[test]
    fn a_project_is_named_by_any_row_that_knows_its_path() {
        let mut nameless = summary("n", "/p/alpha", 40_000);
        nameless.project_path = None;
        let rows = vec![Arc::new(SessionRow::from_summary(&nameless)), row("a", "/p/alpha", 10_000)];
        assert_eq!(project_entries(rows.iter())[0].name, "alpha");
    }

    #[test]
    fn the_conversation_is_the_indexed_texts_without_injected_messages() {
        let msg = |role, text: &str, is_meta| MessageRow {
            uuid: "u".into(),
            parent_uuid: None,
            role,
            ts: 0,
            text: text.into(),
            is_meta,
            byte_offset: 0,
            byte_len: 0,
            model: None,
            input_tokens: None,
            output_tokens: None,
        };
        let messages = vec![
            msg(Role::User, "<caveat>injected</caveat>", true),
            msg(Role::User, "hello", false),
            msg(Role::Assistant, "  ", false),
            msg(Role::Assistant, "hi there", false),
        ];
        let conv = conversation_from(&messages, None);
        let got: Vec<(MessageKind, &str)> = conv.messages.iter().map(|m| (m.kind, m.text.as_ref())).collect();
        assert_eq!(got, [(MessageKind::User, "hello"), (MessageKind::Assistant, "hi there")]);
        assert_eq!(conv.messages[0].time, "", "a record without a timestamp shows no time");
        assert_eq!((conv.total, conv.skipped), (2, 0));

        let many: Vec<MessageRow> = (0..CONVERSATION_CAP + 50).map(|_| msg(Role::User, "x", false)).collect();
        let capped = conversation_from(&many, None);
        assert_eq!((capped.messages.len(), capped.total), (CONVERSATION_CAP, CONVERSATION_CAP + 50));
    }

    fn text_msg(role: Role, text: &str) -> MessageRow {
        MessageRow {
            uuid: "u".into(),
            parent_uuid: None,
            role,
            ts: 0,
            text: text.into(),
            is_meta: false,
            byte_offset: 0,
            byte_len: 0,
            model: None,
            input_tokens: None,
            output_tokens: None,
        }
    }

    #[test]
    fn search_hits_are_marked_in_the_conversation() {
        let messages = vec![text_msg(Role::User, "fix the Parser"), text_msg(Role::Assistant, "done"), text_msg(Role::Assistant, "parser parser")];
        let matcher = Matcher::words("parser").unwrap();
        let conv = conversation_from(&messages, Some(&matcher));
        let marks: Vec<&[Range<usize>]> = conv.messages.iter().map(|m| m.highlights.as_slice()).collect();
        assert_eq!(marks, [&[8..14][..], &[][..], &[0..6, 7..13][..]]);
        assert!(conversation_from(&messages, None).messages.iter().all(|m| m.highlights.is_empty()));
    }

    #[test]
    fn a_hit_beyond_the_cap_moves_the_window_and_keeps_the_turn_numbers() {
        let mut messages: Vec<MessageRow> = (0..CONVERSATION_CAP + 100).map(|i| text_msg(if i % 2 == 0 { Role::User } else { Role::Assistant }, "filler")).collect();
        messages[CONVERSATION_CAP + 40] = text_msg(Role::User, "the needle");
        let matcher = Matcher::words("needle").unwrap();
        let conv = conversation_from(&messages, Some(&matcher));
        assert_eq!(conv.skipped, CONVERSATION_CAP + 40 - 5, "a few messages of context stay ahead of the hit");
        assert_eq!(conv.skipped_turns, conv.skipped.div_ceil(2), "every other skipped message was a prompt");
        assert_eq!(conv.messages.len(), 100 - 40 + 5);
        assert_eq!(conv.messages[5].highlights, vec![4..10]);

        let early = conversation_from(&messages, Some(&Matcher::words("filler").unwrap()));
        assert_eq!(early.skipped, 0, "a hit inside the first window leaves it alone");
    }

    #[test]
    fn a_hit_deep_inside_a_long_message_moves_the_cut() {
        let long = format!("{}needle{}", "a".repeat(10_000), "b".repeat(10_000));
        let matcher = Matcher::words("needle").unwrap();
        let conv = conversation_from(&[text_msg(Role::Assistant, &long)], Some(&matcher));
        let m = &conv.messages[0];
        assert!(m.text.starts_with('…') && m.text.ends_with('…'));
        assert_eq!(m.highlights.len(), 1);
        assert_eq!(&m.text[m.highlights[0].clone()], "needle");
        assert_eq!(m.text.chars().count(), 4000 + 2);
        assert!(conversation_from(&[text_msg(Role::Assistant, &long)], None).messages[0].text.starts_with("aaa"));
    }

    #[test]
    fn a_row_knows_its_transcript_unless_it_was_cleaned_up() {
        let mut s = summary("s", "/p/a", 5000);
        assert_eq!(SessionRow::from_summary(&s).transcript_path.as_deref(), Some(Path::new("/tmp/x.jsonl")));
        s.transcript_missing = true;
        assert_eq!(SessionRow::from_summary(&s).transcript_path, None);
    }
}
