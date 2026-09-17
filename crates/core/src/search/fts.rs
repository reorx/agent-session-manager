//! Text search over the index: FTS5 trigram (case-insensitive substring match, every word must
//! appear) with a `LIKE` fallback for words shorter than a trigram (two-character CJK queries).

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::types::Value as SqlValue;

use super::Highlighted;
use crate::index::Db;
use crate::model::Role;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FtsRoles {
    pub user: bool,
    pub assistant: bool,
}

impl FtsRoles {
    pub fn both() -> Self {
        FtsRoles { user: true, assistant: true }
    }
}

#[derive(Clone, Debug)]
pub struct FtsQuery {
    pub text: String,
    pub roles: FtsRoles,
    /// Case-insensitive substring of the encoded project directory name.
    pub project: Option<String>,
    /// Only messages with `ts >= since_ms`.
    pub since_ms: Option<i64>,
    /// Maximum number of sessions returned.
    pub limit: usize,
}

/// One session with matches, in the order they appear in the transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHit {
    pub session_id: String,
    pub project_key: String,
    pub count: u32,
    pub first: Highlighted,
    pub first_role: Role,
    pub first_ts: i64,
    pub updated_at: i64,
}

/// Normalise user input into FTS5 phrases: every whitespace-separated word becomes a quoted
/// phrase (implicit AND), so operators (`OR`, `-`, `*`, `:`) are matched literally.
pub fn to_fts_query(input: &str) -> String {
    input
        .split_whitespace()
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Trigram FTS returns nothing for words shorter than three characters, so those go to `LIKE`.
pub fn needs_like(input: &str) -> bool {
    input.split_whitespace().any(|w| w.chars().count() < 3)
}

const SNIPPET_CONTEXT_CHARS: usize = 40;

/// Search messages and group the hits per session, most recently updated session first.
pub fn search_sessions(db: &Db, q: &FtsQuery) -> Result<Vec<SessionHit>> {
    let words: Vec<&str> = q.text.split_whitespace().collect();
    let roles: Vec<&str> =
        [(q.roles.user, "user"), (q.roles.assistant, "assistant")].iter().filter(|(on, _)| *on).map(|(_, r)| *r).collect();
    if words.is_empty() || roles.is_empty() {
        return Ok(Vec::new());
    }
    let like = needs_like(&q.text);
    let mut sql = String::from(
        "SELECT m.session_id, p.encoded_dir, m.role, m.ts, s.updated_at, ",
    );
    let mut args: Vec<SqlValue> = Vec::new();
    if like {
        sql.push_str("m.text FROM messages m");
    } else {
        sql.push_str("snippet(messages_fts, 0, char(1), char(2), '…', 64) FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid");
    }
    sql.push_str(" JOIN sessions s ON s.id = m.session_id JOIN projects p ON p.id = s.project_id WHERE ");
    if like {
        sql.push_str("m.is_meta = 0");
        for w in &words {
            args.push(format!("%{}%", escape_like(w)).into());
            sql.push_str(&format!(" AND m.text LIKE ?{} ESCAPE '\\'", args.len()));
        }
    } else {
        args.push(to_fts_query(&q.text).into());
        sql.push_str(&format!("messages_fts MATCH ?{}", args.len()));
    }
    let marks: Vec<String> = roles.iter().map(|r| { args.push(r.to_string().into()); format!("?{}", args.len()) }).collect();
    sql.push_str(&format!(" AND m.role IN ({})", marks.join(",")));
    if let Some(since) = q.since_ms {
        args.push(since.into());
        sql.push_str(&format!(" AND m.ts >= ?{}", args.len()));
    }
    if let Some(project) = q.project.as_deref().filter(|p| !p.is_empty()) {
        args.push(project.to_lowercase().into());
        sql.push_str(&format!(" AND instr(lower(p.encoded_dir), ?{}) > 0", args.len()));
    }
    sql.push_str(" ORDER BY m.session_id, m.byte_offset");

    let mut stmt = db.conn().prepare_cached(&sql)?;
    let mut hits: HashMap<String, SessionHit> = HashMap::new();
    let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    for row in rows {
        let (session_id, project_key, role, ts, updated_at, text) = row?;
        match hits.get_mut(&session_id) {
            Some(hit) => hit.count += 1,
            None => {
                let first = if like { like_snippet(&text, &words) } else { parse_markers(&text) };
                hits.insert(
                    session_id.clone(),
                    SessionHit {
                        session_id,
                        project_key,
                        count: 1,
                        first,
                        first_role: Role::parse(&role).unwrap_or(Role::User),
                        first_ts: ts,
                        updated_at,
                    },
                );
            }
        }
    }
    let mut out: Vec<SessionHit> = hits.into_values().collect();
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.session_id.cmp(&b.session_id)));
    out.truncate(q.limit);
    Ok(out)
}

fn escape_like(w: &str) -> String {
    w.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Turn FTS5's `char(1)` / `char(2)` markers into highlight ranges.
fn parse_markers(marked: &str) -> Highlighted {
    let mut out = Highlighted::default();
    let mut open: Option<usize> = None;
    for c in marked.chars() {
        match c {
            '\u{1}' => open = Some(out.text.len()),
            '\u{2}' => {
                if let Some(start) = open.take() {
                    out.ranges.push(start..out.text.len());
                }
            }
            c => out.text.push(c),
        }
    }
    out
}

/// Snippet for the LIKE path: context around the first word, every word occurrence highlighted.
/// SQLite's LIKE folds ASCII case only, so the same rule is used here.
fn like_snippet(text: &str, words: &[&str]) -> Highlighted {
    let lower = text.to_ascii_lowercase();
    let first = words.iter().filter_map(|w| lower.find(&w.to_ascii_lowercase())).min().unwrap_or(0);
    let start = text[..first].char_indices().rev().nth(SNIPPET_CONTEXT_CHARS - 1).map(|(i, _)| i).unwrap_or(0);
    let end = text[first..].char_indices().nth(SNIPPET_CONTEXT_CHARS * 2).map(|(i, _)| first + i).unwrap_or(text.len());
    let mut out = Highlighted::default();
    if start > 0 {
        out.text.push('…');
    }
    let base = out.text.len();
    let shown = text[start..end].replace('\n', "⏎");
    let shown_lower = shown.to_ascii_lowercase();
    for w in words {
        let wl = w.to_ascii_lowercase();
        let mut pos = 0;
        while let Some(i) = shown_lower[pos..].find(&wl) {
            out.ranges.push(base + pos + i..base + pos + i + wl.len());
            pos += i + wl.len();
        }
    }
    out.text.push_str(&shown);
    out.ranges.sort_by_key(|r| r.start);
    out.ranges.dedup();
    if end < text.len() {
        out.text.push('…');
    }
    out
}
