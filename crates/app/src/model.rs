//! View-model types for the UI. Everything the table renders is pre-formatted into
//! `SharedString`s so `render_td` never allocates.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_session_core::format::{fmt_datetime, fmt_time, fmt_tokens, fmt_usd};
use agent_session_core::pricing::context_window;
use agent_session_core::model::SessionSummary;
use chrono::{Datelike, Days, Local, Months, NaiveDate};
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
    pub favorite: bool,
    pub favorite_path: SharedString,
    pub id: SharedString,
    /// False for a history session from before Claude Code recorded session ids: `id` is made up.
    pub id_known: bool,
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
    /// Priced from token usage, subagents included.
    pub cost: Option<f64>,
    pub cost_fmt: SharedString,
    /// What Claude Code recorded itself (`cost-state`), for comparison.
    pub cc_cost_fmt: SharedString,
    /// Last main-thread prompt as a share of the model's context window.
    pub context_pct: Option<f32>,
    /// `41%`, `<1%` or a dash.
    pub context_fmt: SharedString,
    /// `412k / 1M · peak 655k`.
    pub context_detail: SharedString,
    pub tokens_fmt: SharedString,
    /// Models whose calls are left out of `cost` (no known price); empty when there are none.
    pub unpriced: SharedString,
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
    /// Files the session changed: the project's relative to it (sorted), then the others.
    pub files_touched: Vec<SharedString>,
    /// `12 files`, or a dash.
    pub files_fmt: SharedString,
    pub subagents_fmt: SharedString,
    pub pr_url: SharedString,
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

fn format_cost(usd: Option<f64>) -> SharedString {
    usd.map_or(NONE.into(), |c| fmt_usd(c).into())
}

fn format_context(pct: Option<f32>) -> SharedString {
    match pct {
        Some(p) if p < 0.01 => "<1%".into(),
        Some(p) => format!("{:.0}%", p * 100.0).into(),
        None => NONE.into(),
    }
}

fn context_detail(s: &SessionSummary) -> SharedString {
    let Some(tokens) = s.context_tokens else { return NONE.into() };
    let mut text = fmt_tokens(tokens);
    if let Some(window) = s.context_model.as_deref().and_then(context_window) {
        text.push_str(&format!(" / {}", fmt_tokens(window)));
    }
    if let Some(peak) = s.context_peak {
        text.push_str(&format!(" · peak {}", fmt_tokens(peak)));
    }
    text.into()
}

fn tokens_summary(s: &SessionSummary) -> SharedString {
    let t = fmt_tokens;
    format!("in {} · out {} · cache read {} · write {}", t(s.tokens_in), t(s.tokens_out), t(s.tokens_cache_read), t(s.tokens_cache_write)).into()
}

fn project_name(s: &SessionSummary) -> String {
    let path = s.project_path.as_deref().or(s.cwd.as_deref());
    match path.and_then(|p| Path::new(p).file_name()) {
        Some(name) => name.to_string_lossy().into_owned(),
        None => s.project_key.clone(),
    }
}

/// `files_touched` with the project's files relative to it, listed first. Claude Code records a
/// file under the session's working directory relative to it already, and others absolute.
fn files_touched(s: &SessionSummary) -> Vec<SharedString> {
    let project = s.project_path.as_deref().or(s.cwd.as_deref()).map(|p| p.trim_end_matches('/')).filter(|p| !p.is_empty());
    let (mut inside, mut outside): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    for path in &s.files_touched {
        match project.and_then(|p| path.strip_prefix(p)?.strip_prefix('/')) {
            Some(rel) => inside.push(rel),
            None if !path.starts_with('/') => inside.push(path),
            None => outside.push(path),
        }
    }
    // the others come sorted from the BTreeSet
    inside.sort_unstable();
    inside.dedup();
    inside.into_iter().chain(outside).map(|p| SharedString::from(p.to_string())).collect()
}

fn files_fmt(n: usize) -> SharedString {
    match n {
        0 => NONE.into(),
        1 => "1 file".into(),
        n => format!("{n} files").into(),
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
        let context_pct = s.context_ratio().map(|r| r as f32);
        let files = files_touched(s);
        SessionRow {
            favorite: s.favorite,
            favorite_path: if s.favorite { s.file_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into() } else { NONE.into() },
            id: s.id.clone().into(),
            id_known: s.id_known,
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
            cost: s.cost_usd,
            cost_fmt: format_cost(s.cost_usd),
            cc_cost_fmt: format_cost(s.total_cost_usd),
            context_pct,
            context_fmt: format_context(context_pct),
            context_detail: context_detail(s),
            tokens_fmt: tokens_summary(s),
            unpriced: s.unpriced_models.iter().cloned().collect::<Vec<_>>().join(", ").into(),
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
            files_fmt: files_fmt(files.len()),
            files_touched: files,
            subagents_fmt: s.subagent_count.to_string().into(),
            pr_url: or_none(s.pr_url.as_deref()),
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
    ThisMonth,
    LastMonth,
    /// A picked start and end day, both included.
    Custom,
}

impl RangePreset {
    /// Menu order; a separator goes before `Custom`.
    pub const ALL: [RangePreset; 8] = [
        RangePreset::Today,
        RangePreset::Days3,
        RangePreset::Days7,
        RangePreset::Days30,
        RangePreset::Any,
        RangePreset::ThisMonth,
        RangePreset::LastMonth,
        RangePreset::Custom,
    ];

    /// Button value.
    pub fn label(self) -> &'static str {
        match self {
            RangePreset::Today => "Today",
            RangePreset::Days3 => "3 days",
            RangePreset::Days7 => "7 days",
            RangePreset::Days30 => "30 days",
            RangePreset::Any => "Any time",
            RangePreset::ThisMonth => "This month",
            RangePreset::LastMonth => "Last month",
            RangePreset::Custom => "Custom",
        }
    }

    pub fn menu_label(self) -> &'static str {
        match self {
            RangePreset::Custom => "Custom range…",
            other => other.label(),
        }
    }

    /// The range Custom starts with: this month up to today.
    pub fn default_custom(today: NaiveDate) -> (NaiveDate, NaiveDate) {
        (today.with_day(1).unwrap_or(today), today)
    }

    /// `[since, until)` for `updated_at` (ms since epoch, local days), `None` for an open end.
    pub fn bounds(self, custom: Option<(NaiveDate, NaiveDate)>) -> (Option<i64>, Option<i64>) {
        self.bounds_on(Local::now().date_naive(), custom)
    }

    pub fn bounds_on(self, today: NaiveDate, custom: Option<(NaiveDate, NaiveDate)>) -> (Option<i64>, Option<i64>) {
        let days_back = |days: u64| (local_midnight(today.checked_sub_days(Days::new(days)).unwrap_or(today)), None);
        let month = today.with_day(1).unwrap_or(today);
        match self {
            RangePreset::Today => days_back(0),
            RangePreset::Days3 => days_back(2),
            RangePreset::Days7 => days_back(6),
            RangePreset::Days30 => days_back(29),
            RangePreset::Any => (None, None),
            RangePreset::ThisMonth => (local_midnight(month), None),
            RangePreset::LastMonth => (month.checked_sub_months(Months::new(1)).and_then(local_midnight), local_midnight(month)),
            RangePreset::Custom => {
                let (start, end) = custom.unwrap_or_else(|| Self::default_custom(today));
                (local_midnight(start), end.succ_opt().and_then(local_midnight))
            }
        }
    }
}

/// Start of `date` in local time (the earlier one when a DST change repeats midnight).
fn local_midnight(date: NaiveDate) -> Option<i64> {
    Some(date.and_hms_opt(0, 0, 0)?.and_local_timezone(Local).earliest()?.timestamp_millis())
}

/// Filter applied to the table rows before sorting.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Filter {
    pub favorites_only: bool,
    /// `project_key` of the only project to show.
    pub project: Option<SharedString>,
    pub since_ms: Option<i64>,
    /// Exclusive upper bound for `updated_at`.
    pub until_ms: Option<i64>,
    /// Sessions that never got a human prompt are hidden unless this is set.
    pub show_empty: bool,
    pub kinds: KindSet,
    /// `project_key`s whose sessions never show (noise such as a probe tool's directory).
    pub hidden_projects: BTreeSet<SharedString>,
}

impl Filter {
    pub fn matches(&self, row: &SessionRow) -> bool {
        let project_ok = self.project.as_ref().is_none_or(|p| row.project_key == *p);
        let since_ok = self.since_ms.is_none_or(|since| row.updated_at >= since)
            && self.until_ms.is_none_or(|until| row.updated_at < until);
        let shown = (self.show_empty || row.prompts > 0) && !self.hidden_projects.contains(&row.project_key);
        (!self.favorites_only || row.favorite) && project_ok && since_ok && shown && self.kinds.contains(row.kind)
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
        assert_eq!((r.cost_fmt.as_ref(), r.cost), ("—", None));
        assert_eq!((r.cc_cost_fmt.as_ref(), r.context_fmt.as_ref(), r.context_pct), ("—", "—", None));
        assert_eq!(r.context_detail, "—");
        assert_eq!((r.short_id.as_ref(), r.id_known), ("(unknown)", false));
    }

    #[test]
    fn sizes_durations_and_tools_are_human_readable() {
        let mut s = summary("s", "/p/a", 5000);
        s.file_size = Some(3 * 1024 * 1024 + 512 * 1024);
        s.duration_ms = Some(3_900_000);
        s.cost_usd = Some(1.966);
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
    fn a_row_lists_the_files_it_touched_relative_to_the_project() {
        let mut s = summary("s", "/Users/me/Code/alpha", 5000);
        // Claude Code records a file under the working directory relative to it, others absolute
        s.files_touched = ["/Users/me/Code/alpha/src/parse.rs", "README.md", "/Users/me/notes/todo.md", "/Users/me/Code/alphabet/x.rs", "crates/app/src/main.rs", "AGENTS.md"]
            .map(String::from)
            .into();
        let r = SessionRow::from_summary(&s);
        assert_eq!(r.files_fmt, "6 files");
        assert_eq!(
            r.files_touched,
            ["AGENTS.md", "README.md", "crates/app/src/main.rs", "src/parse.rs", "/Users/me/Code/alphabet/x.rs", "/Users/me/notes/todo.md"],
            "the project's files first, relative and sorted; then the others as they are"
        );

        s.files_touched = ["/Users/me/Code/alpha/a.rs", "a.rs"].map(String::from).into();
        let r = SessionRow::from_summary(&s);
        assert_eq!((r.files_fmt.as_ref(), r.files_touched.as_slice()), ("1 file", &["a.rs".into()][..]), "recorded both ways, one file");
        s.files_touched.clear();
        let r = SessionRow::from_summary(&s);
        assert_eq!((r.files_fmt.as_ref(), r.files_touched.len()), ("—", 0), "no file changed: a dash");
    }

    #[test]
    fn a_row_knows_its_transcript_unless_it_was_cleaned_up() {
        let mut s = summary("s", "/p/a", 5000);
        assert_eq!(SessionRow::from_summary(&s).transcript_path.as_deref(), Some(Path::new("/tmp/x.jsonl")));
        s.transcript_missing = true;
        assert_eq!(SessionRow::from_summary(&s).transcript_path, None);
    }

    #[test]
    fn cost_is_the_priced_one_and_claude_codes_figure_is_kept_for_comparison() {
        let mut s = summary("s", "/p/a", 5000);
        (s.cost_usd, s.total_cost_usd) = (Some(3.4219), Some(3.5));
        (s.tokens_in, s.tokens_out, s.tokens_cache_read, s.tokens_cache_write) = (12, 1050, 89_000, 2900);
        let r = SessionRow::from_summary(&s);
        assert_eq!((r.cost, r.cost_fmt.as_ref(), r.cc_cost_fmt.as_ref()), (Some(3.4219), "$3.42", "$3.50"));
        assert_eq!(r.tokens_fmt, "in 12 · out 1k · cache read 89k · write 3k");
        assert_eq!(r.unpriced, "");
        s.unpriced_models = ["claude-opus-9".to_string()].into();
        assert_eq!(SessionRow::from_summary(&s).unpriced, "claude-opus-9");
    }

    #[test]
    fn context_is_a_share_of_the_models_window() {
        let mut s = summary("s", "/p/a", 5000);
        (s.context_tokens, s.context_peak, s.context_model) = (Some(412_000), Some(655_000), Some("claude-opus-5".into()));
        let r = SessionRow::from_summary(&s);
        assert_eq!((r.context_fmt.as_ref(), r.context_detail.as_ref()), ("41%", "412k / 1M · peak 655k"));
        assert!((r.context_pct.unwrap() - 0.412).abs() < 1e-6);
        s.context_tokens = Some(4_000);
        assert_eq!(SessionRow::from_summary(&s).context_fmt, "<1%");
        (s.context_tokens, s.context_model) = (Some(150_000), Some("claude-haiku-4-5-20251001".into()));
        assert_eq!(SessionRow::from_summary(&s).context_fmt, "75%", "Haiku 4.5 has a 200K window");
        s.context_model = Some("claude-opus-9".into());
        let r = SessionRow::from_summary(&s);
        assert_eq!((r.context_fmt.as_ref(), r.context_pct), ("—", None), "no window, no share");
        assert_eq!(r.context_detail, "150k · peak 655k");
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn midnight(date: NaiveDate) -> Option<i64> {
        Some(date.and_hms_opt(0, 0, 0).unwrap().and_local_timezone(Local).earliest().unwrap().timestamp_millis())
    }

    #[test]
    fn day_presets_have_only_a_lower_bound() {
        let today = day(2026, 10, 23);
        assert_eq!(RangePreset::Today.bounds_on(today, None), (midnight(today), None));
        assert_eq!(RangePreset::Days7.bounds_on(today, None), (midnight(day(2026, 10, 17)), None), "today and the 6 days before");
        assert_eq!(RangePreset::Any.bounds_on(today, None), (None, None));
    }

    #[test]
    fn month_presets_cover_whole_calendar_months() {
        let today = day(2026, 10, 23);
        assert_eq!(RangePreset::ThisMonth.bounds_on(today, None), (midnight(day(2026, 10, 1)), None));
        assert_eq!(RangePreset::LastMonth.bounds_on(today, None), (midnight(day(2026, 9, 1)), midnight(day(2026, 10, 1))));
        assert_eq!(
            RangePreset::LastMonth.bounds_on(day(2026, 1, 5), None),
            (midnight(day(2025, 12, 1)), midnight(day(2026, 1, 1))),
            "January looks back into last year"
        );
    }

    #[test]
    fn a_custom_range_includes_its_last_day_and_defaults_to_this_month_so_far() {
        let today = day(2026, 10, 23);
        let range = (day(2026, 9, 10), day(2026, 9, 20));
        assert_eq!(RangePreset::Custom.bounds_on(today, Some(range)), (midnight(day(2026, 9, 10)), midnight(day(2026, 9, 21))));
        assert_eq!(RangePreset::default_custom(today), (day(2026, 10, 1), today));
        assert_eq!(RangePreset::Custom.bounds_on(today, None), (midnight(day(2026, 10, 1)), midnight(day(2026, 10, 24))));
        assert_eq!(RangePreset::Days3.bounds_on(today, Some(range)), (midnight(day(2026, 10, 21)), None), "only Custom reads the range");
    }

    #[test]
    fn the_filter_has_an_exclusive_upper_bound() {
        let filter = Filter { since_ms: Some(10_000), until_ms: Some(20_000), ..Default::default() };
        assert!(filter.matches(&row("a", "/p/a", 10_000)), "the lower bound is in");
        assert!(filter.matches(&row("b", "/p/a", 19_999)));
        assert!(!filter.matches(&row("c", "/p/a", 20_000)), "the upper bound is the next day's midnight: out");
        assert!(!filter.matches(&row("d", "/p/a", 9_999)));
    }

    #[test]
    fn the_range_button_reads_custom_while_its_menu_entry_invites_to_pick() {
        assert_eq!(RangePreset::Custom.label(), "Custom");
        assert_eq!(RangePreset::Custom.menu_label(), "Custom range…");
        assert_eq!(RangePreset::LastMonth.menu_label(), "Last month");
    }
}
