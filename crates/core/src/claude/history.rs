//! `~/.claude/history.jsonl`: every prompt ever typed, with project path and (since 2025-11)
//! session id. Never cleaned up, so it recovers sessions whose transcript is gone.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::clean::clean_prompt;
use super::paths::{ClaudeDir, encode_project_path};
pub use crate::model::HistoryPrompt;
use crate::model::SessionSummary;

/// Rows without a session id (pre 2025-11) are split into sessions on this gap.
pub const HISTORY_GAP_MS: i64 = 2 * 3600 * 1000;

#[derive(Clone, Debug, Deserialize)]
pub struct HistoryRow {
    pub display: String,
    #[serde(rename = "pastedContents", default)]
    pub pasted: Map<String, Value>,
    pub timestamp: i64,
    pub project: String,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
}

/// Rows grouped by encoded project name, in file (chronological) order. A missing file is empty.
pub fn load_history(path: &Path) -> Result<BTreeMap<String, Vec<HistoryRow>>> {
    let mut by_project: BTreeMap<String, Vec<HistoryRow>> = BTreeMap::new();
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(by_project),
        Err(e) => return Err(e.into()),
    };
    for line in bytes.split(|b| *b == b'\n') {
        let Ok(row) = serde_json::from_slice::<HistoryRow>(line) else { continue };
        if row.project.is_empty() {
            continue;
        }
        by_project.entry(encode_project_path(&row.project)).or_default().push(row);
    }
    Ok(by_project)
}

static PASTE_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[Pasted text #(\d+)(?: \+\d+ lines)?\]").unwrap());

/// Put pasted text back into a row's `display`, which only keeps `[Pasted text #N +M lines]`.
/// The text is inline in `pastedContents`, or (newer rows) in `paste-cache/<contentHash>.txt`;
/// placeholders that cannot be resolved are kept as they are.
pub fn expand_pasted(display: &str, pasted: &Map<String, Value>, cache_dir: &Path) -> String {
    PASTE_REF
        .replace_all(display, |caps: &regex::Captures| {
            let item = pasted.get(&caps[1]);
            if let Some(content) = item.and_then(|i| i.get("content")).and_then(Value::as_str) {
                return content.to_string();
            }
            if let Some(hash) = item.and_then(|i| i.get("contentHash")).and_then(Value::as_str)
                && let Ok(text) = std::fs::read(cache_dir.join(format!("{hash}.txt")))
            {
                return String::from_utf8_lossy(&text).into_owned();
            }
            caps[0].to_string()
        })
        .into_owned()
}

/// One chronological history group. Borrow rows instead of copying pasted content.
#[derive(Debug)]
pub struct HistoryGroup<'a> {
    pub id: String,
    pub id_known: bool,
    pub rows: Vec<&'a HistoryRow>,
}

/// Use the same grouping for table summaries and detail prompts.
pub fn group_history<'a>(project_key: &str, rows: &'a [HistoryRow]) -> Vec<HistoryGroup<'a>> {
    let mut groups: Vec<HistoryGroup<'a>> = Vec::new();
    let mut by_id: BTreeMap<&str, usize> = BTreeMap::new();
    let mut anon: Option<usize> = None;
    for row in rows {
        match row.session_id.as_deref().filter(|s| !s.is_empty()) {
            Some(sid) => {
                let ix = *by_id.entry(sid).or_insert_with(|| {
                    groups.push(HistoryGroup { id: sid.to_string(), id_known: true, rows: Vec::new() });
                    groups.len() - 1
                });
                groups[ix].rows.push(row);
            }
            None => {
                let start_new = match anon {
                    Some(ix) => row.timestamp - groups[ix].rows.last().unwrap().timestamp > HISTORY_GAP_MS,
                    None => true,
                };
                if start_new {
                    groups.push(HistoryGroup { id: format!("history:{project_key}:{}", row.timestamp), id_known: false, rows: Vec::new() });
                    anon = Some(groups.len() - 1);
                }
                groups[anon.unwrap()].rows.push(row);
            }
        }
    }
    groups
}

/// Read one group's prompts in file order, restoring pastes before cleaning text.
/// Missing history, projects or session IDs return an empty list, as does load_history.
pub fn history_prompts(claude: &ClaudeDir, project_key: &str, session_id: &str) -> Result<Vec<HistoryPrompt>> {
    let projects = load_history(&claude.history_file())?;
    let Some(rows) = projects.get(project_key) else { return Ok(Vec::new()); };
    let cache = claude.paste_cache_dir();
    Ok(group_history(project_key, rows).into_iter()
        .find(|g| g.id == session_id)
        .map(|g| g.rows.into_iter().map(|r| HistoryPrompt {
            ts: r.timestamp,
            text: clean_prompt(&expand_pasted(&r.display, &r.pasted, &cache)),
        }).collect()).unwrap_or_default())
}

/// Group one project's rows (chronological) into sessions, like the Python reference.
pub fn history_sessions(project_key: &str, rows: &[HistoryRow], cache_dir: &Path) -> Vec<SessionSummary> {
    group_history(project_key, rows)
        .into_iter()
        .map(|HistoryGroup { id, id_known: known, rows: grp }| {
            let first = grp.iter().find(|r| !r.display.starts_with('/')).unwrap_or(&grp[0]);
            let last = grp.last().unwrap();
            let mut s = SessionSummary::history(&id, known, project_key, &grp[0].project);
            s.first_prompt = clean_prompt(&expand_pasted(&first.display, &first.pasted, cache_dir));
            s.last_prompt = Some(clean_prompt(&expand_pasted(&last.display, &last.pasted, cache_dir)));
            s.user_msg_count = grp.len() as u32;
            s.started_at = Some(grp[0].timestamp);
            s.updated_at = last.timestamp;
            s
        })
        .collect()
}
