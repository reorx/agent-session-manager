//! Read independent billing, main-thread and rebuild facts; never join these one-to-many tables.
use std::collections::HashMap;
use anyhow::Result;
use chrono::{Local, TimeZone};
use rusqlite::params;
use crate::{model::*, pricing::normalize};
use super::Db;

impl Db {
    /// Query only index facts. `sessions` supplies the selected IDs and project labels;
    /// its metadata time and main model never filter the calls.
    pub fn query_cache(&self, query: &CacheQuery, sessions: &[SessionSummary]) -> Result<CacheReport> {
        let selected: HashMap<&str, &SessionSummary> = sessions.iter().map(|s| (s.id.as_str(), s)).collect();
        let mut out = CacheReport::default();
        let mut usage = self.conn.prepare_cached("SELECT session_id, model, NULLIF(ts,0), input, cache_write_5m,
            cache_write_1h, cache_read, output, web_search FROM api_usage
            WHERE (?1 IS NULL OR ts >= ?1) AND (?2 IS NULL OR ts < ?2)
            AND (ts != 0 OR (?1 IS NULL AND ?2 IS NULL))")?;
        let mut rows = usage.query(params![query.since_ms, query.until_ms])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let model: String = r.get(1)?;
            let ts: Option<i64> = r.get(2)?;
            let Some(s) = selected.get(id.as_str()).filter(|_| matches_model(query, &model)) else { continue };
            let tokens = Tokens { input: r.get::<_, i64>(3)? as u64, cache_write_5m: r.get::<_, i64>(4)? as u64, cache_write_1h: r.get::<_, i64>(5)? as u64,
                cache_read: r.get::<_, i64>(6)? as u64, output: r.get::<_, i64>(7)? as u64, web_search: r.get::<_, i64>(8)? as u64 };
            visit(&mut out, query, s, &model, ts, |stats| { stats.calls += 1; stats.tokens.add(&tokens); });
        }
        let mut calls = self.conn.prepare_cached("SELECT session_id, model, ts FROM cache_calls
            WHERE (?1 IS NULL OR ts >= ?1) AND (?2 IS NULL OR ts < ?2)")?;
        let mut rows = calls.query(params![query.since_ms, query.until_ms])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let model: String = r.get(1)?;
            let Some(s) = selected.get(id.as_str()).filter(|_| matches_model(query, &model)) else { continue };
            visit(&mut out, query, s, &model, r.get(2)?, |stats| stats.main_thread_calls += 1);
        }
        let mut breaks = self.conn.prepare_cached("SELECT session_id, model, cause, data FROM cache_breaks
            WHERE (?1 IS NULL OR ts >= ?1) AND (?2 IS NULL OR ts < ?2)")?;
        let mut rows = breaks.query(params![query.since_ms, query.until_ms])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let model: String = r.get(1)?;
            let Some(s) = selected.get(id.as_str()).filter(|_| matches_model(query, &model)) else { continue };
            let cause: String = r.get(2)?;
            let b: IndexedCacheBreak = serde_json::from_str(&r.get::<_, String>(3)?)?;
            visit(&mut out, query, s, &model, b.rebuilt.ts, |stats| stats.add_break(&cause, &b.rebuilt));
        }
        out.totals.finish();
        for stats in out.sessions.values_mut().chain(out.groups.values_mut()) { stats.finish(); }
        Ok(out)
    }
}

fn matches_model(q: &CacheQuery, model: &str) -> bool {
    q.model.as_deref().is_none_or(|m| normalize(m).trim_start_matches("claude-") == normalize(model).trim_start_matches("claude-"))
}

fn visit(out: &mut CacheReport, q: &CacheQuery, s: &SessionSummary, model: &str, ts: Option<i64>, mut add: impl FnMut(&mut CacheStats)) {
    add(&mut out.totals);
    add(out.sessions.entry(s.id.clone()).or_default());
    if let Some(by) = q.group {
        let key = match by {
            CacheGroup::Day => ts.and_then(|ts| Local.timestamp_millis_opt(ts).single()).map(|t| t.format("%Y-%m-%d").to_string()),
            CacheGroup::Project => Some(s.project_path.as_deref().unwrap_or(&s.project_key).to_string()),
            CacheGroup::Model => Some(normalize(model).to_string()),
        };
        add(out.groups.entry(key).or_default());
    }
}

impl CacheStats {
    fn add_break(&mut self, code: &str, b: &CacheBreak) {
        self.rebuilds += 1;
        self.written += b.written;
        self.cost_usd = Some(self.cost_usd.unwrap_or(0.0) + b.cost_usd.unwrap_or(0.0));
        self.unpriced_rebuilds += u64::from(b.cost_usd.is_none());
        let cause = self.causes.entry(code.to_string()).or_default();
        cause.rebuilds += 1;
        cause.written += b.written;
        cause.cost_usd = Some(cause.cost_usd.unwrap_or(0.0) + b.cost_usd.unwrap_or(0.0));
        cause.unpriced_rebuilds += u64::from(b.cost_usd.is_none());
        if let CacheBreakCause::Idle { gap_ms, .. } = b.cause { cause.gaps.push(gap_ms); }
    }
    fn finish(&mut self) {
        let t = &self.tokens;
        self.input_tokens = t.input_total();
        self.hit_rate = ratio(t.cache_read, self.input_tokens);
        self.write_rate = ratio(t.cache_write_5m + t.cache_write_1h, self.input_tokens);
        self.uncached_rate = ratio(t.input, self.input_tokens);
        self.write_1h_share = ratio(t.cache_write_1h, t.cache_write_5m + t.cache_write_1h);
        self.cost_usd = (self.unpriced_rebuilds == 0).then_some(self.cost_usd.unwrap_or(0.0));
        for c in self.causes.values_mut() {
            c.cost_usd = (c.unpriced_rebuilds == 0).then_some(c.cost_usd.unwrap_or(0.0));
            c.gaps.sort_unstable();
            let n = c.gaps.len();
            c.median_gap_ms = (n > 0).then(|| (c.gaps[(n - 1) / 2] as f64 + c.gaps[n / 2] as f64) / 2.0);
        }
    }
}
fn ratio(n: u64, d: u64) -> Option<f64> { (d > 0).then(|| n as f64 / d as f64) }
