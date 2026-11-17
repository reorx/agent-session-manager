//! Cache overview and lifetime details; all statistics come from core index queries.
use std::{cmp::Ordering, collections::HashMap, io::Write, time::Instant};
use agent_session_core::{claude::paths::home_dir, format::fmt_usd, model::*, pricing::normalize,
    search::sessions::{ProjectMatcher, SessionFilter, select_sessions}};
use anyhow::Result;
use clap::{Args, ValueEnum};
use serde_json::{Value, json};
use crate::{App, agent::{RangeArgs, LookupError, excluded_session, now_ms, project_name, rfc3339, title, usage}};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Group { Day, Project, Model }
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Sort { RebuildCost, Hit, Tokens }
impl Sort {
    fn name(self) -> &'static str { match self { Self::RebuildCost => "rebuild-cost", Self::Hit => "hit", Self::Tokens => "tokens" } }
}
impl Group {
    fn name(self) -> &'static str { match self { Self::Day => "day", Self::Project => "project", Self::Model => "model" } }
    fn core(self) -> CacheGroup { match self { Self::Day => CacheGroup::Day, Self::Project => CacheGroup::Project, Self::Model => CacheGroup::Model } }
}
#[derive(Args)]
pub struct CacheArgs {
    /// Inspect one session's entire lifetime (unique prefix accepted)
    #[arg(value_name = "ID", conflicts_with_all = ["days", "since", "until", "projects", "model", "include_self", "by", "sort", "limit"])]
    id: Option<String>,
    #[command(flatten)]
    range: RangeArgs,
    /// Project directory or substring (repeatable)
    #[arg(short = 'p', long = "project", value_name = "P", allow_hyphen_values = true)]
    projects: Vec<String>,
    /// Normalized model of each call, including subagents
    #[arg(long, value_name = "M")]
    model: Option<String>,
    /// Include CLAUDE_CODE_SESSION_ID in the overview
    #[arg(long)]
    include_self: bool,
    /// Add a group table
    #[arg(long, value_enum)]
    by: Option<Group>,
    /// Session ranking (default: rebuild-cost)
    #[arg(long, value_enum)]
    sort: Option<Sort>,
    /// Maximum ranked sessions (default 10; 0 is unlimited)
    #[arg(short = 'n', long, value_name = "N")]
    limit: Option<usize>,
    /// Include main-thread calls in lifetime details
    #[arg(long, requires = "id")]
    calls: bool,
    /// One JSON object
    #[arg(long)]
    json: bool,
    /// Skip refresh when the index has already been scanned
    #[arg(long)]
    no_scan: bool,
}

impl App {
    pub(crate) fn cache(&mut self, args: CacheArgs) -> Result<()> {
        let result = self.cache_inner(&args);
        self.fail_as_json(args.json, result)
    }
    fn cache_inner(&mut self, a: &CacheArgs) -> Result<()> {
        let started = Instant::now();
        if a.id.as_ref().is_some_and(|s| s.trim().is_empty()) { return Err(usage("ID is empty")); }
        let range = a.range.resolve(now_ms(), a.id.is_none().then_some(7))?;
        let excluded = if a.id.is_none() { excluded_session(a.include_self) } else { None };
        let (db, refresh) = self.open_db_refreshed(!a.no_scan)?;
        // One read snapshot, including metadata, all three aggregates and lifetime details.
        let _snapshot = db.conn().unchecked_transaction()?;
        let sessions = if let Some(id) = &a.id {
            let mut found = db.find_by_prefix(id)?;
            match found.len() {
                0 => return Err(LookupError::NotFound(id.clone()).into()),
                1 => found,
                _ => {
                    found.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
                    return Err(LookupError::Ambiguous { prefix: id.clone(), candidates: found }.into());
                }
            }
        } else {
            let cwd = crate::agent::current_dir_for(&a.projects)?;
            select_sessions(&db, &SessionFilter {
                projects: a.projects.iter().map(|p| ProjectMatcher::parse(p, &cwd, &home_dir())).collect(),
                exclude: excluded.clone(), ..Default::default()
            })?
        };
        let model = a.model.as_deref().map(|m| normalize(m).to_string());
        let report = db.query_cache(&CacheQuery { since_ms: range.since_ms, until_ms: range.until_ms,
            model: model.clone(), group: a.by.map(Group::core) }, &sessions)?;
        let metadata: HashMap<&str, &SessionSummary> = sessions.iter().map(|s| (s.id.as_str(), s)).collect();
        let sort = a.sort.unwrap_or(Sort::RebuildCost);
        let limit = a.limit.unwrap_or(10);
        let mut ranked: Vec<_> = report.sessions.iter().collect();
        ranked.sort_by(|(ai, av), (bi, bv)| {
            let order = match sort {
                Sort::RebuildCost => optional_number(av.cost_usd, bv.cost_usd, true),
                Sort::Hit => optional_number(av.hit_rate, bv.hit_rate, false),
                Sort::Tokens => bv.input_tokens.cmp(&av.input_tokens),
            };
            order.then_with(|| metadata[bi.as_str()].updated_at.cmp(&metadata[ai.as_str()].updated_at)).then(ai.cmp(bi))
        });
        let total = if a.id.is_some() { 1 } else { ranked.len() };
        if limit > 0 { ranked.truncate(limit); }
        let results: Vec<Value> = if a.id.is_some() { vec![] } else { ranked.iter().map(|(id, s)| {
            let mut v = session_json(metadata[id.as_str()]); v["stats"] = stats_json(s); v
        }).collect() };
        let shown = if a.id.is_some() { 1 } else { results.len() };
        let mut groups: Vec<_> = report.groups.iter().collect();
        groups.sort_by(|(a, _), (b, _)| a.is_none().cmp(&b.is_none()).then(a.cmp(b)));
        let groups: Vec<_> = groups.into_iter().map(|(key, s)| json!({"key":key,"stats":stats_json(s)})).collect();
        let session = a.id.as_ref().map(|_| session_json(&sessions[0]));
        let breaks = if a.id.is_some() { db.cache_breaks(&sessions[0].id)? } else { vec![] };
        let calls = if a.calls { db.cache_calls(&sessions[0].id)? } else { vec![] };
        let rebuilds: Vec<_> = breaks.iter().map(break_json).collect();
        let calls: Vec<_> = calls.iter().map(|c| json!({"msg_id":c.facts.msg_id,"offset":c.byte_offset,"turn":c.turn,
            "time":c.facts.ts.and_then(rfc3339),"model":normalize(&c.facts.model),"prompt_tokens":c.facts.prompt,
            "prompt_cache_read":c.facts.read,"tokens":c.facts.tokens,"rebuild":breaks.iter().any(|b| b.msg_id == c.facts.msg_id)})).collect();
        let value = json!({"meta":{"command":"cache","query":a.id,"range":range.json(),"projects":a.projects,
            "model":model,"excluded_session":excluded,"by":a.by.map(Group::name),
            "sort":a.id.is_none().then_some(sort.name()),"limit":a.id.is_none().then_some(limit),
            "index_refresh":refresh.as_str(),"elapsed_ms":started.elapsed().as_millis(),"total":total,"shown":shown,"truncated":shown<total},
            "totals":stats_json(&report.totals),"groups":groups,"results":results,"session":session,"rebuilds":rebuilds,"calls":calls});
        if a.json { self.write_json(&value) } else { self.cache_text(&value, &report.totals, &range.label) }
    }

    fn cache_text(&mut self, v: &Value, s: &CacheStats, range: &str) -> Result<()> {
        writeln!(self.out, "{range} · {} sessions · {} main-thread calls", v["meta"]["total"], s.main_thread_calls)?;
        if !v["session"].is_null() { self.cache_session_line(&v["session"], None)?; }
        if s.calls == 0 && s.main_thread_calls == 0 { writeln!(self.out, "0 calls in range")?; }
        writeln!(self.out, "input {} tokens: cache read {} · cache write {} (1h {}) · uncached {}", s.input_tokens,
            percent(s.hit_rate), percent(s.write_rate), percent(s.write_1h_share), percent(s.uncached_rate))?;
        writeln!(self.out, "rebuilds {} · {} tokens written · {}", s.rebuilds, s.written, money(s.cost_usd))?;
        for (cause, c) in &s.causes {
            let gap = c.median_gap_ms.map(|n| format!(" · median gap {:.1}m", n / 60000.0)).unwrap_or_default();
            writeln!(self.out, "  {cause}  {} · {}{gap}", c.rebuilds, money(c.cost_usd))?;
        }
        for g in v["groups"].as_array().unwrap() {
            let s = &g["stats"];
            writeln!(self.out, "{}  hit {} · write {} · rebuilds {} · {}", string(&g["key"]),
                percent(s["hit_rate"].as_f64()), percent(s["write_rate"].as_f64()), s["rebuilds"], money(s["cost_usd"].as_f64()))?;
        }
        if !v["results"].as_array().unwrap().is_empty() { writeln!(self.out, "top sessions by {}:", string(&v["meta"]["sort"]))?; }
        for r in v["results"].as_array().unwrap() { self.cache_session_line(r, Some(&r["stats"]))?; }
        if v["meta"]["truncated"] == true { writeln!(self.out, "showing {} of {} sessions, use -n for more", v["meta"]["shown"], v["meta"]["total"])?; }
        for b in v["rebuilds"].as_array().unwrap() {
            writeln!(self.out, "turn {} · {} · {} · dropped {} · written {} · {}", number(&b["turn"]), string(&b["time"]),
                cause_text(&b["cause"]), b["dropped"], b["written"], money(b["cost_usd"].as_f64()))?;
        }
        for c in v["calls"].as_array().unwrap() {
            let t = &c["tokens"];
            writeln!(self.out, "{} · turn {} · {} · prompt {} · read {} · write 5m {} / 1h {} · uncached {}{}", string(&c["time"]),
                number(&c["turn"]), string(&c["model"]), c["prompt_tokens"], c["prompt_cache_read"], t["cache_write_5m"], t["cache_write_1h"],
                t["input"], if c["rebuild"] == true { " · rebuild" } else { "" })?;
        }
        Ok(())
    }
    fn cache_session_line(&mut self, r: &Value, stats: Option<&Value>) -> Result<()> {
        let metrics = stats.map(|s| format!(" · hit {} · rebuilds {} · {}", percent(s["hit_rate"].as_f64()), s["rebuilds"], money(s["cost_usd"].as_f64()))).unwrap_or_default();
        writeln!(self.out, "{}  {}  {}{metrics}  {}", string(&r["session_id"]), string(&r["updated_at"]), string(&r["project"]), string(&r["title"]))?;
        if stats.is_none() { writeln!(self.out, "started {} · updated {}", string(&r["started_at"]), string(&r["updated_at"]))?; }
        Ok(())
    }
}

fn optional_number(a: Option<f64>, b: Option<f64>, descending: bool) -> Ordering {
    match (a, b) { (Some(a), Some(b)) => if descending { b.total_cmp(&a) } else { a.total_cmp(&b) },
        (None, Some(_)) => Ordering::Greater, (Some(_), None) => Ordering::Less, _ => Ordering::Equal }
}
fn session_json(s: &SessionSummary) -> Value {
    json!({"session_id":s.id,"title":title(s),"project":project_name(s),"project_path":s.project_path,
        "started_at":s.started_at.and_then(rfc3339),"updated_at":rfc3339(s.updated_at)})
}
fn stats_json(s: &CacheStats) -> Value {
    let mut v = serde_json::to_value(s).expect("cache statistics serialize");
    v["causes"] = s.causes.iter().map(|(code, stats)| { let mut v = serde_json::to_value(stats).unwrap(); v["cause"] = code.clone().into(); v }).collect();
    v
}
fn break_json(b: &IndexedCacheBreak) -> Value {
    let cause = match &b.rebuilt.cause {
        CacheBreakCause::EffortChange { from, to } => json!({"kind":"effort","from":from,"to":to}),
        CacheBreakCause::RequestSettings { sign_in } => json!({"kind":if *sign_in {"sign_in"} else {"request_settings"}}),
        c => serde_json::to_value(c).unwrap(),
    };
    json!({"msg_id":b.msg_id,"offset":b.byte_offset,"turn":b.turn,"time":b.rebuilt.ts.and_then(rfc3339),
        "model":normalize(&b.model),"dropped":b.rebuilt.dropped,"written":b.rebuilt.written,"cost_usd":b.rebuilt.cost_usd,"cause":cause})
}
fn cause_text(c: &Value) -> String {
    let kind = string(&c["kind"]);
    match kind {
        "idle" => format!("idle · gap {}ms / TTL {}ms", c["gap_ms"], c["ttl_ms"]),
        "model_switch" => format!("model_switch from {}", string(&c["from"])),
        "effort" => format!("effort {} → {}", string(&c["from"]), string(&c["to"])),
        "fast_mode" => format!("fast_mode on={}", c["on"]),
        _ => kind.to_string(),
    }
}
fn string(v: &Value) -> &str { v.as_str().unwrap_or("—") }
fn number(v: &Value) -> String { if v.is_null() { "—".into() } else { v.to_string() } }
fn percent(v: Option<f64>) -> String { v.map(|n| format!("{:.1}%", n * 100.0)).unwrap_or_else(|| "—".into()) }
fn money(v: Option<f64>) -> String { v.map(fmt_usd).unwrap_or_else(|| "unpriced".into()) }
