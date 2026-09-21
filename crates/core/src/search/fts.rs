//! Text search over the index: FTS5 trigram (case-insensitive substring match, every word must
//! appear) with a `LIKE` fallback for words shorter than a trigram (two-character CJK queries).

use std::collections::HashSet;

use anyhow::Result;
use rusqlite::types::Value as SqlValue;

use super::{Highlighted, Matcher};
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
    /// Only these sessions (`None` = all of them).
    pub sessions: Option<HashSet<String>>,
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

/// The newest `limit` sessions with hits, plus what the whole query matched.
#[derive(Clone, Debug, Default)]
pub struct FtsOutcome {
    pub hits: Vec<SessionHit>,
    pub total_sessions: usize,
    pub total_matches: usize,
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

/// Characters shown before the first matched word / from its start.
const SNIPPET_BEFORE: usize = 40;
const SNIPPET_AFTER: usize = 80;

/// Search messages and group the hits per session, most recently updated session first.
pub fn search_sessions(db: &Db, q: &FtsQuery) -> Result<Vec<SessionHit>> {
    Ok(search(db, q)?.hits)
}

/// Two steps: SQL only counts the hits per session (FTS5's own `snippet()` re-tokenizes every
/// matching message, 50x the cost of the query), then the first matching message of each
/// session that is kept gets its snippet here.
pub fn search(db: &Db, q: &FtsQuery) -> Result<FtsOutcome> {
    let Some(matcher) = Matcher::words(&q.text) else { return Ok(FtsOutcome::default()) };
    let mut hits = count_per_session(db, q)?;
    if let Some(sessions) = &q.sessions {
        hits.retain(|(hit, _)| sessions.contains(&hit.session_id));
    }
    let total_sessions = hits.len();
    let total_matches = hits.iter().map(|(hit, _)| hit.count as usize).sum();
    hits.sort_by(|(a, _), (b, _)| b.updated_at.cmp(&a.updated_at).then_with(|| a.session_id.cmp(&b.session_id)));
    hits.truncate(q.limit);

    let mut text_of = db.conn().prepare_cached("SELECT text FROM messages WHERE id = ?1")?;
    let mut out = Vec::with_capacity(hits.len());
    for (mut hit, message_id) in hits {
        let text: String = text_of.query_row([message_id], |r| r.get(0))?;
        hit.first = matcher.snippet(&text, SNIPPET_BEFORE, SNIPPET_AFTER);
        out.push(hit);
    }
    Ok(FtsOutcome { hits: out, total_sessions, total_matches })
}

/// Every session with hits (snippet still empty) and the id of its first matching message.
fn count_per_session(db: &Db, q: &FtsQuery) -> Result<Vec<(SessionHit, i64)>> {
    let words: Vec<&str> = q.text.split_whitespace().collect();
    let roles: Vec<&str> =
        [(q.roles.user, "user"), (q.roles.assistant, "assistant")].iter().filter(|(on, _)| *on).map(|(_, r)| *r).collect();
    if words.is_empty() || roles.is_empty() {
        return Ok(Vec::new());
    }
    let like = needs_like(&q.text);
    // min() makes the bare columns (id, role, ts) come from the first message in conversation order
    let mut sql = String::from("SELECT m.session_id, p.encoded_dir, s.updated_at, count(*), min(m.byte_offset), m.id, m.role, m.ts FROM ");
    let mut args: Vec<SqlValue> = Vec::new();
    sql.push_str(if like { "messages m" } else { "messages_fts JOIN messages m ON m.id = messages_fts.rowid" });
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
    sql.push_str(" GROUP BY m.session_id");

    let mut stmt = db.conn().prepare_cached(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
        let hit = SessionHit {
            session_id: r.get(0)?,
            project_key: r.get(1)?,
            updated_at: r.get(2)?,
            count: r.get(3)?,
            first: Highlighted::default(),
            first_role: Role::parse(&r.get::<_, String>(6)?).unwrap_or(Role::User),
            first_ts: r.get(7)?,
        };
        Ok((hit, r.get::<_, i64>(5)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn escape_like(w: &str) -> String {
    w.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}
