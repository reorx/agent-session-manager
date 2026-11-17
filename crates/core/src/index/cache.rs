//! Cache facts are inserted and replayed on the index's single writer, inside its transaction.
use anyhow::Result;
use rusqlite::params;
use serde::de::DeserializeOwned;
use crate::{claude::cache_events::replay, model::*};
use super::Db;

impl Db {
    pub fn insert_cache_events(&self, session: &str, events: &[CacheEvent]) -> Result<()> {
        let mut stmt = self.conn.prepare_cached("INSERT INTO cache_events(session_id, byte_offset, data) VALUES (?1, ?2, ?3)")?;
        for event in events { stmt.execute(params![session, i64::try_from(event.byte_offset)?, serde_json::to_string(event)?])?; }
        Ok(())
    }

    pub fn delete_cache_facts(&self, session: &str) -> Result<()> {
        for table in ["cache_events", "cache_calls", "cache_breaks"] {
            self.conn.execute(&format!("DELETE FROM {table} WHERE session_id = ?1"), [session])?;
        }
        Ok(())
    }

    pub fn cache_events(&self, session: &str) -> Result<Vec<CacheEvent>> {
        self.cache_rows("cache_events", session)
    }

    pub fn cache_calls(&self, session: &str) -> Result<Vec<IndexedCacheCall>> {
        self.cache_rows("cache_calls", session)
    }

    pub fn cache_breaks(&self, session: &str) -> Result<Vec<IndexedCacheBreak>> {
        self.cache_rows("cache_breaks", session)
    }

    fn cache_rows<T: DeserializeOwned>(&self, table: &str, session: &str) -> Result<Vec<T>> {
        let mut stmt = self.conn.prepare_cached(&format!("SELECT data FROM {table} WHERE session_id = ?1 ORDER BY byte_offset"))?;
        let rows = stmt.query_map([session], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    /// Recompute derived rows exclusively from persisted events, never from transcript files.
    /// The caller must hold a write transaction, as for refresh_session_usage.
    pub fn refresh_cache_breaks(&self, session: &str) -> Result<()> {
        let (calls, breaks) = replay(&self.cache_events(session)?);
        self.conn.execute("DELETE FROM cache_calls WHERE session_id = ?1", [session])?;
        self.conn.execute("DELETE FROM cache_breaks WHERE session_id = ?1", [session])?;
        let mut call_stmt = self.conn.prepare_cached("INSERT INTO cache_calls
            (session_id, msg_id, byte_offset, ts, turn, model, parent_msg_id, prompt_tokens, prompt_cache_read, data)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)")?;
        for call in calls {
            let c = &call.facts;
            call_stmt.execute(params![session, c.msg_id, i64::try_from(call.byte_offset)?, c.ts, call.turn, c.model,
                call.parent_msg_id, i64::try_from(c.prompt)?, i64::try_from(c.read)?, serde_json::to_string(&call)?])?;
        }
        let mut break_stmt = self.conn.prepare_cached("INSERT INTO cache_breaks
            (session_id, msg_id, byte_offset, ts, turn, model, dropped, written, cost_usd, cause, data)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)")?;
        for row in breaks {
            let b = &row.rebuilt;
            break_stmt.execute(params![session, row.msg_id, i64::try_from(row.byte_offset)?, b.ts, row.turn, row.model,
                i64::try_from(b.dropped)?, i64::try_from(b.written)?, b.cost_usd, cause_code(&b.cause), serde_json::to_string(&row)?])?;
        }
        Ok(())
    }
}

fn cause_code(cause: &CacheBreakCause) -> &'static str {
    match cause {
        CacheBreakCause::Compaction => "compaction",
        CacheBreakCause::ModelSwitch { .. } => "model_switch",
        CacheBreakCause::EffortChange { .. } => "effort",
        CacheBreakCause::FastMode { .. } => "fast_mode",
        CacheBreakCause::RequestSettings { sign_in: true } => "sign_in",
        CacheBreakCause::RequestSettings { sign_in: false } => "request_settings",
        CacheBreakCause::Idle { .. } => "idle",
        CacheBreakCause::SystemOrTools => "system_or_tools",
        CacheBreakCause::EarlierMessages => "earlier_messages",
        CacheBreakCause::RequestUnchanged => "request_unchanged",
        CacheBreakCause::Unknown => "unknown",
    }
}
