//! What the commands for agents (`search`, `session`, `projects`) share: the time range, the
//! current session, how a session is titled and resumed, times in JSON, and failures in JSON.
//! The conventions are in `skills/asm/references/cli.md` ("Conventions").

use std::io::Write;
use std::path::Path;

use agent_session_core::claude::clean::one_line;
use agent_session_core::claude::paths::ClaudeDir;
use agent_session_core::favorites::original_exists;
use agent_session_core::format::fmt_datetime;
use agent_session_core::model::SessionSummary;
use agent_session_core::search::Highlighted;
use agent_session_core::session::resume_command;
use anyhow::Result;
use chrono::{Local, NaiveDate, SecondsFormat, TimeZone};
use clap::Args;
use serde_json::{Value, json};

use crate::{App, Palette};

/// Characters of a title in text output.
pub const TITLE_LEN: usize = 120;
/// Characters of a prompt in JSON.
pub const JSON_PROMPT_LEN: usize = 2000;
/// Shown for a session with neither a title nor a prompt.
pub const NO_PROMPT: &str = "(no prompt)";

/// Invalid arguments found after clap parsed them (exit 2).
#[derive(Debug)]
pub struct UsageError(pub String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

pub fn usage(e: impl std::fmt::Display) -> anyhow::Error {
    UsageError(e.to_string()).into()
}

// ---------------------------------------------------------------------------------------------
// Time range

/// `-d N | --since DATE [--until DATE]`.
#[derive(Args, Debug)]
pub struct RangeArgs {
    /// Activity within the last N days (N × 24 hours, decimals allowed); 0 for all time
    #[arg(short = 'd', long, value_name = "N", conflicts_with_all = ["since", "until"])]
    days: Option<f64>,
    /// Activity on or after local midnight of DATE (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    since: Option<NaiveDate>,
    /// Activity on or before the end of local day DATE (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    until: Option<NaiveDate>,
}

/// The range a command used: activity overlapping `[since_ms, until_ms)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    pub label: String,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    /// No range option was given.
    pub default: bool,
}

impl Range {
    pub fn limited(&self) -> bool {
        self.since_ms.is_some() || self.until_ms.is_some()
    }

    pub fn json(&self) -> Value {
        json!({
            "label": self.label,
            "since": self.since_ms.and_then(rfc3339),
            "until": self.until_ms.and_then(rfc3339),
            "default": self.default,
        })
    }
}

impl RangeArgs {
    /// The range, or the command's default without a range option: the last `default_days`
    /// days, or all time for `None`.
    pub fn resolve(&self, now_ms: i64, default_days: Option<u32>) -> Result<Range> {
        let all_time = |default| Range { label: "all time".into(), since_ms: None, until_ms: None, default };
        if let Some(days) = self.days {
            if !days.is_finite() || days < 0.0 {
                return Err(usage(format!("--days must be a number of days, 0 or more (got {days})")));
            }
            if days == 0.0 {
                return Ok(all_time(false));
            }
            let since = now_ms - (days * 86_400_000.0) as i64;
            return Ok(Range { label: format!("last {}", days_label(days)), since_ms: Some(since), until_ms: None, default: false });
        }
        if let (Some(since), Some(until)) = (self.since, self.until)
            && since > until
        {
            return Err(usage(format!("--since {since} is later than --until {until}")));
        }
        let since_ms = self.since.map(local_midnight).transpose()?;
        let until_ms = self.until.map(|d| local_midnight(d + chrono::Days::new(1))).transpose()?;
        let label = match (self.since, self.until) {
            (Some(s), Some(u)) => format!("{s} to {u}"),
            (Some(s), None) => format!("since {s}"),
            (None, Some(u)) => format!("until {u}"),
            (None, None) => {
                return Ok(match default_days {
                    Some(days) => Range {
                        label: format!("last {} (default)", days_label(days as f64)),
                        since_ms: Some(now_ms - days as i64 * 86_400_000),
                        until_ms: None,
                        default: true,
                    },
                    None => all_time(true),
                });
            }
        };
        Ok(Range { label, since_ms, until_ms, default: false })
    }
}

/// `7 days`, `1 day`, `0.5 days`.
fn days_label(days: f64) -> String {
    match days == 1.0 {
        true => "1 day".into(),
        false => format!("{days} days"),
    }
}

fn local_midnight(date: NaiveDate) -> Result<i64> {
    let midnight = date.and_hms_opt(0, 0, 0).expect("midnight exists");
    match Local.from_local_datetime(&midnight).earliest() {
        Some(dt) => Ok(dt.timestamp_millis()),
        None => Err(usage(format!("{date} has no local midnight"))),
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// RFC 3339 with the local offset, to the second.
pub fn rfc3339(ms: i64) -> Option<String> {
    Local.timestamp_millis_opt(ms).single().map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, false))
}

// ---------------------------------------------------------------------------------------------
// The current session

/// The session to leave out: the one `asm` runs inside (`CLAUDE_CODE_SESSION_ID`), unless
/// `--include-self`.
pub fn excluded_session(include_self: bool) -> Option<String> {
    let id = std::env::var("CLAUDE_CODE_SESSION_ID").ok().filter(|id| !id.trim().is_empty())?;
    (!include_self).then_some(id)
}

// ---------------------------------------------------------------------------------------------
// Sessions

/// The session's title, else the first line of its first prompt; `None` when both are empty.
pub fn title(s: &SessionSummary) -> Option<String> {
    let title = s.display_title().trim();
    (!title.is_empty()).then(|| one_line(title, TITLE_LEN))
}

/// The session has a title of its own (not its first prompt).
pub fn has_own_title(s: &SessionSummary) -> bool {
    s.title.as_deref().is_some_and(|t| !t.trim().is_empty())
}

/// The project directory, else the working directory.
pub fn project_dir(s: &SessionSummary) -> Option<&str> {
    s.project_path.as_deref().or(s.cwd.as_deref())
}

/// Last component of the project directory, else the encoded directory name.
pub fn project_name(s: &SessionSummary) -> String {
    match project_dir(s).and_then(|p| Path::new(p).file_name()) {
        Some(name) => name.to_string_lossy().into_owned(),
        None => s.project_key.clone(),
    }
}

/// Only the copy kept by favoriting is left of the transcript.
pub fn favorite_copy_only(s: &SessionSummary, claude: &ClaudeDir) -> bool {
    s.favorite && !s.history_only() && !original_exists(claude, &s.project_key, &s.id)
}

/// The command that resumes the session in Claude Code, or why there is none.
pub fn resume(s: &SessionSummary, claude: &ClaudeDir) -> Result<String, &'static str> {
    if s.history_only() {
        return Err("history only");
    }
    if favorite_copy_only(s, claude) {
        return Err("only a favorite copy is left");
    }
    let dir = project_dir(s).unwrap_or("");
    let exists = Path::new(dir).is_absolute() && Path::new(dir).is_dir();
    resume_command(&s.id, dir, exists).ok_or("no plain session id")
}

/// `resume: …` in text output.
pub fn resume_text(s: &SessionSummary, claude: &ClaudeDir) -> String {
    match resume(s, claude) {
        Ok(cmd) => cmd,
        Err(why) => format!("none ({why})"),
    }
}

/// At most `max` characters, `…` when cut.
pub fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// A prompt for JSON: `null` when empty.
pub fn json_prompt(text: Option<&str>) -> Value {
    match text.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => cut(t, JSON_PROMPT_LEN).into(),
        None => Value::Null,
    }
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Matched text between `«` and `»`, also bold red with colors.
pub fn marked(h: &Highlighted, pal: &Palette) -> String {
    h.render(&format!("«{}", pal.hl), &format!("{}»", pal.reset))
}

// ---------------------------------------------------------------------------------------------
// Looking a session up, and failures

/// `asm session ID` matched no session, or several.
#[derive(Debug)]
pub enum LookupError {
    NotFound(String),
    /// Most recently updated first.
    Ambiguous { prefix: String, candidates: Vec<SessionSummary> },
}

/// Candidates listed on stderr.
const CANDIDATES_SHOWN: usize = 20;

impl LookupError {
    /// One line: what `error.message` carries in JSON.
    pub fn message(&self) -> String {
        match self {
            LookupError::NotFound(prefix) => format!("no session matches '{prefix}'"),
            LookupError::Ambiguous { prefix, candidates } => format!("prefix '{prefix}' matches {} sessions:", candidates.len()),
        }
    }
}

/// The message, and for an ambiguous prefix the candidates, one per line.
impl std::fmt::Display for LookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())?;
        if let LookupError::Ambiguous { candidates, .. } = self {
            for s in candidates.iter().take(CANDIDATES_SHOWN) {
                let title = title(s).unwrap_or_else(|| NO_PROMPT.into());
                write!(f, "\n  {}  {}  {title}", s.id, fmt_datetime(Some(s.updated_at)))?;
            }
            if candidates.len() > CANDIDATES_SHOWN {
                write!(f, "\n  … {} more", candidates.len() - CANDIDATES_SHOWN)?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for LookupError {}

impl App {
    pub(crate) fn write_json(&mut self, value: &Value) -> Result<()> {
        serde_json::to_writer_pretty(&mut self.out, value)?;
        writeln!(self.out)?;
        Ok(())
    }

    /// With `--json`, a failure (exit 1) also goes to stdout as `{ "error": … }`; stderr gets
    /// the same message. Invalid arguments (exit 2) only go to stderr.
    pub(crate) fn fail_as_json(&mut self, json: bool, result: Result<()>) -> Result<()> {
        let Err(e) = result else { return Ok(()) };
        if !json || e.is::<UsageError>() {
            return Err(e);
        }
        let (error, message) = match e.downcast_ref::<LookupError>() {
            Some(LookupError::NotFound(_)) => {
                let message = e.to_string();
                (json!({ "code": "not_found", "message": message }), message)
            }
            Some(lookup @ LookupError::Ambiguous { candidates, .. }) => {
                let message = lookup.message();
                let candidates: Vec<Value> = candidates
                    .iter()
                    .map(|s| json!({ "session_id": s.id, "updated_at": rfc3339(s.updated_at), "title": title(s) }))
                    .collect();
                (json!({ "code": "ambiguous", "message": message, "candidates": candidates }), message)
            }
            None => {
                let message = e.to_string();
                (json!({ "code": "failed", "message": message }), message)
            }
        };
        self.write_json(&json!({ "error": error }))?;
        Err(anyhow::anyhow!(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(days: Option<f64>, since: Option<&str>, until: Option<&str>) -> RangeArgs {
        RangeArgs { days, since: since.map(|d| d.parse().unwrap()), until: until.map(|d| d.parse().unwrap()) }
    }

    #[test]
    fn range_labels_name_what_was_used() {
        let now = 1_800_000_000_000;
        let label = |r: RangeArgs, default| r.resolve(now, default).unwrap().label;
        assert_eq!(label(range(Some(7.0), None, None), None), "last 7 days");
        assert_eq!(label(range(Some(1.0), None, None), None), "last 1 day");
        assert_eq!(label(range(Some(0.5), None, None), None), "last 0.5 days");
        assert_eq!(label(range(Some(0.0), None, None), Some(30)), "all time", "--days 0 removes the default too");
        assert_eq!(label(range(None, None, None), Some(30)), "last 30 days (default)");
        assert_eq!(label(range(None, None, None), None), "all time");
        assert_eq!(label(range(None, Some("2026-11-01"), None), Some(30)), "since 2026-11-01");
        assert_eq!(label(range(None, Some("2026-11-01"), Some("2026-11-14")), None), "2026-11-01 to 2026-11-14");
        assert_eq!(label(range(None, None, Some("2026-11-14")), None), "until 2026-11-14");
        let r = range(Some(2.0), None, None).resolve(now, None).unwrap();
        assert_eq!((r.since_ms, r.until_ms, r.default), (Some(now - 2 * 86_400_000), None, false));
        assert!(range(None, None, None).resolve(now, None).unwrap().default);
    }

    #[test]
    fn until_covers_its_whole_local_day() {
        let r = range(None, Some("2026-11-01"), Some("2026-11-01")).resolve(0, None).unwrap();
        assert_eq!(r.until_ms.unwrap() - r.since_ms.unwrap(), 86_400_000);
    }

    #[test]
    fn a_range_that_ends_before_it_starts_or_counts_back_is_invalid() {
        let e = range(None, Some("2026-11-14"), Some("2026-11-01")).resolve(0, None).unwrap_err();
        assert_eq!(e.to_string(), "--since 2026-11-14 is later than --until 2026-11-01");
        assert!(e.is::<UsageError>());
        assert!(range(Some(-1.0), None, None).resolve(0, None).unwrap_err().is::<UsageError>());
    }

    #[test]
    fn cut_counts_characters() {
        assert_eq!(cut("连不上网络", 3), "连不上…");
        assert_eq!(cut("abc", 3), "abc");
    }
}
