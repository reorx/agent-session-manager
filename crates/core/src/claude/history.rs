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
use super::paths::encode_project_path;
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

/// Group one project's rows (chronological) into sessions, like the reference `history_sessions`.
pub fn history_sessions(project_key: &str, rows: &[HistoryRow], cache_dir: &Path) -> Vec<SessionSummary> {
    let mut groups: Vec<(String, bool, Vec<&HistoryRow>)> = Vec::new();
    let mut by_id: BTreeMap<&str, usize> = BTreeMap::new();
    let mut anon: Option<usize> = None;
    for row in rows {
        match row.session_id.as_deref().filter(|s| !s.is_empty()) {
            Some(sid) => {
                let ix = *by_id.entry(sid).or_insert_with(|| {
                    groups.push((sid.to_string(), true, Vec::new()));
                    groups.len() - 1
                });
                groups[ix].2.push(row);
            }
            None => {
                let start_new = match anon {
                    Some(ix) => row.timestamp - groups[ix].2.last().unwrap().timestamp > HISTORY_GAP_MS,
                    None => true,
                };
                if start_new {
                    groups.push((format!("history:{project_key}:{}", row.timestamp), false, Vec::new()));
                    anon = Some(groups.len() - 1);
                }
                groups[anon.unwrap()].2.push(row);
            }
        }
    }
    groups
        .into_iter()
        .map(|(id, known, grp)| {
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
