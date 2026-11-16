//! `search`: find sessions by their content, for agents. The CLI only maps the options onto the
//! core's session search (`search::sessions`) and renders what it finds, as text or JSON.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use agent_session_core::claude::clean::one_line;
use agent_session_core::claude::paths::home_dir;
use agent_session_core::cost::primary_model;
use agent_session_core::format::{fmt_datetime, fmt_time, fmt_usd};
use agent_session_core::model::SessionSummary;
use agent_session_core::search::query::Query;
use agent_session_core::search::sessions::{
    Engine, Hit, Kind, KindSet, ProjectMatcher, SearchOptions, SearchOutcome, SessionFilter, SessionHits, SortBy, search, select_sessions,
};
use agent_session_core::session::SessionKind;
use anyhow::Result;
use clap::{Args, ValueEnum};
use serde_json::{Value, json};

use crate::agent::{
    NO_PROMPT, Range, RangeArgs, excluded_session, has_own_title, json_prompt, marked, now_ms, plural, project_name, resume, resume_text,
    rfc3339, title, usage,
};
use crate::{App, Refresh};

/// Without a range option, tool kinds and regexes look at this many days.
const SCAN_DEFAULT_DAYS: u32 = 30;
/// Characters of the first prompt in text output.
const PROMPT_LEN: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum SortArg {
    Recent,
    Hits,
}

#[derive(Args)]
pub struct SearchArgs {
    /// A concept the session must contain; `|` separates ways of saying it (put `--` before a
    /// TERM that starts with `-`)
    #[arg(value_name = "TERM", required = true)]
    terms: Vec<String>,
    /// Text kinds to search, comma-separated: prompt, reply, thinking, tool, tool-input,
    /// tool-output, other, all (default: prompt,reply,thinking; tool with --tool)
    #[arg(long = "in", value_name = "KINDS")]
    kinds: Option<String>,
    /// Only calls of this tool (exact name, case-insensitive; repeatable)
    #[arg(long = "tool", value_name = "NAME")]
    tools: Vec<String>,
    /// Each TERM is a regular expression
    #[arg(long)]
    regex: bool,
    /// All TERMs in one item (message, tool call or tool result)
    #[arg(long)]
    same_message: bool,
    #[command(flatten)]
    range: RangeArgs,
    /// Only these projects: a directory, or text in project paths (repeatable). The next word is
    /// the value even when it starts with `-`, as an encoded directory name does
    #[arg(short = 'p', long = "project", value_name = "P", allow_hyphen_values = true)]
    projects: Vec<String>,
    /// Only sessions whose main model is this, e.g. opus-5-5
    #[arg(long, value_name = "M")]
    model: Option<String>,
    /// Only favorite sessions
    #[arg(long)]
    favorites: bool,
    /// Include the session asm runs inside (CLAUDE_CODE_SESSION_ID)
    #[arg(long)]
    include_self: bool,
    /// recent: last activity first; hits: most hits first
    #[arg(long, value_enum, default_value_t = SortArg::Recent)]
    sort: SortArg,
    /// At most N sessions; 0 for no limit
    #[arg(short = 'n', long, value_name = "N", default_value_t = 20)]
    limit: usize,
    /// At most N snippets per session; 0 for none
    #[arg(long, value_name = "N", default_value_t = 3)]
    snippets: usize,
    /// JSON output
    #[arg(long)]
    json: bool,
    /// Do not refresh the index first
    #[arg(long)]
    no_scan: bool,
}

/// Everything a rendering needs.
struct Found<'a> {
    args: &'a SearchArgs,
    query: &'a Query,
    kinds: &'a KindSet,
    range: Range,
    excluded: Option<String>,
    /// `--project` was given and selects no session at all, at any time.
    no_such_project: bool,
    refresh: Refresh,
    outcome: SearchOutcome,
    sessions: HashMap<&'a str, &'a SessionSummary>,
    elapsed_ms: u128,
}

impl App {
    pub(crate) fn find_sessions(&mut self, args: SearchArgs) -> Result<()> {
        let json = args.json;
        let result = self.find_sessions_inner(&args);
        self.fail_as_json(json, result)
    }

    fn find_sessions_inner(&mut self, args: &SearchArgs) -> Result<()> {
        let started = Instant::now();
        let query = Query::parse(&args.terms, args.regex, args.same_message).map_err(usage)?;
        let kinds = KindSet::parse(args.kinds.as_deref(), &args.tools).map_err(usage)?;
        let default_days = (Engine::for_query(&query, &kinds) == Engine::Scan).then_some(SCAN_DEFAULT_DAYS);
        let range = args.range.resolve(now_ms(), default_days)?;
        for i in query.regex_fallback() {
            eprintln!("term {} is not a valid regular expression, matched as literal text", i + 1);
        }
        let excluded = excluded_session(args.include_self);
        let (cwd, home) = (current_dir_for(&args.projects)?, home_dir());
        let filter = SessionFilter {
            projects: args.projects.iter().map(|p| ProjectMatcher::parse(p, &cwd, &home)).collect(),
            since_ms: range.since_ms,
            until_ms: range.until_ms,
            model: args.model.clone(),
            favorites: args.favorites,
            exclude: excluded.clone(),
        };
        let (db, refresh) = self.open_db_refreshed(!args.no_scan)?;
        let selected = select_sessions(&db, &filter)?;
        let sort = match args.sort {
            SortArg::Recent => SortBy::Recent,
            SortArg::Hits => SortBy::Hits,
        };
        let opts = SearchOptions { snippets: args.snippets, sort, limit: args.limit, ..SearchOptions::default() };
        let outcome = search(&db, &selected, &query, &kinds, &opts)?;
        let no_such_project = outcome.total == 0
            && !filter.projects.is_empty()
            && select_sessions(&db, &SessionFilter { projects: filter.projects.clone(), ..SessionFilter::default() })?.is_empty();
        let sessions = selected.iter().map(|s| (s.id.as_str(), s)).collect();
        let found = Found {
            args,
            query: &query,
            kinds: &kinds,
            range,
            excluded,
            no_such_project,
            refresh,
            outcome,
            sessions,
            elapsed_ms: started.elapsed().as_millis(),
        };
        match args.json {
            true => self.search_json(&found),
            false => self.search_text(&found),
        }
    }

    fn search_text(&mut self, f: &Found) -> Result<()> {
        let (total, shown) = (f.outcome.total, f.outcome.sessions.len());
        let mut head = plural(total, "session", "sessions");
        if shown < total {
            head.push_str(&format!(" (showing {shown})"));
        }
        let terms: Vec<String> = f.query.terms().iter().map(|t| format!("[{}]", t.alternatives.join("|"))).collect();
        head.push_str(&format!(" · terms {}", terms.join(" ")));
        if f.query.regex {
            head.push_str(" · regex");
        }
        if f.query.same_message {
            head.push_str(" · same message");
        }
        let kinds: Vec<&str> = f.kinds.kinds().iter().map(|k| k.as_str()).collect();
        head.push_str(&format!(" · in {}", kinds.join(",")));
        if !f.args.tools.is_empty() {
            head.push_str(&format!(" · tools {}", f.args.tools.join(",")));
        }
        if !f.args.projects.is_empty() {
            head.push_str(&format!(" · project {}", f.args.projects.join(",")));
        }
        if let Some(m) = &f.args.model {
            head.push_str(&format!(" · model {m}"));
        }
        if f.args.favorites {
            head.push_str(" · favorites");
        }
        head.push_str(&format!(" · {} · {:.2}s", f.range.label, f.elapsed_ms as f64 / 1000.0));
        writeln!(self.out, "{head}")?;
        if total == 0 {
            writeln!(self.out, "try: {}", hints(f).join(" · "))?;
            return Ok(());
        }
        for hits in &f.outcome.sessions {
            let s = f.sessions[hits.session_id.as_str()];
            writeln!(self.out)?;
            self.session_text(s, hits)?;
        }
        if shown < total {
            writeln!(self.out, "\nshowing {shown} of {total} sessions, use -n for more")?;
        }
        Ok(())
    }

    fn session_text(&mut self, s: &SessionSummary, hits: &SessionHits) -> Result<()> {
        writeln!(self.out, "{}{}{}", self.pal.bold, s.id, self.pal.reset)?;
        let mut line = format!("updated {}", fmt_datetime(Some(s.updated_at)));
        if let Some(started) = s.started_at {
            line.push_str(&format!(" · started {}", fmt_datetime(Some(started))));
        }
        line.push_str(&format!(" · {}", project_name(s)));
        if let Some(branch) = s.git_branch.as_deref().filter(|b| !b.is_empty()) {
            line.push_str(&format!(" ({branch})"));
        }
        line.push_str(&format!(" · {}", plural(hits.turns as usize, "turn", "turns")));
        if let Some(cost) = s.cost_usd {
            line.push_str(&format!(" · {}", fmt_usd(cost)));
        }
        writeln!(self.out, "  {line}")?;
        writeln!(self.out, "  title: {}", title(s).unwrap_or_else(|| NO_PROMPT.into()))?;
        if has_own_title(s) && !s.first_prompt.trim().is_empty() {
            writeln!(self.out, "  prompt: {}", one_line(s.first_prompt.trim(), PROMPT_LEN))?;
        }
        let counts: Vec<String> = hits.counts.iter().map(|(k, n)| format!("{} {n}", k.as_str())).collect();
        writeln!(self.out, "  hits: {}", counts.join(" · "))?;
        for hit in &hits.snippets {
            writeln!(self.out, "  [{}] {}", snippet_label(hit), marked(&hit.text, &self.pal))?;
        }
        writeln!(self.out, "  resume: {}", resume_text(s, &self.claude))?;
        Ok(())
    }

    fn search_json(&mut self, f: &Found) -> Result<()> {
        let (total, shown) = (f.outcome.total, f.outcome.sessions.len());
        let scan = f.outcome.scan.as_ref().map(|s| {
            json!({ "files": s.files, "skipped_files": s.skipped_files, "bytes": s.bytes, "skipped_bytes": s.skipped_bytes })
        });
        let meta = json!({
            "command": "search",
            "terms": f.query.terms().iter().map(|t| t.alternatives.clone()).collect::<Vec<_>>(),
            "regex": f.query.regex,
            "regex_fallback": f.query.regex_fallback(),
            "same_message": f.query.same_message,
            "kinds": f.kinds.kinds().iter().map(|k| k.as_str()).collect::<Vec<_>>(),
            "tools": f.args.tools,
            "range": f.range.json(),
            "projects": f.args.projects,
            "model": f.args.model,
            "favorites": f.args.favorites,
            "excluded_session": f.excluded,
            "sort": match f.args.sort { SortArg::Recent => "recent", SortArg::Hits => "hits" },
            "engine": f.outcome.engine.as_str(),
            "scan": scan,
            "index_refresh": f.refresh.as_str(),
            "elapsed_ms": f.elapsed_ms,
            "total": total,
            "shown": shown,
            "truncated": shown < total,
            "hints": if total == 0 { hints(f) } else { Vec::new() },
        });
        let results: Vec<Value> = f.outcome.sessions.iter().map(|h| self.result_json(f.sessions[h.session_id.as_str()], h)).collect();
        self.write_json(&json!({ "meta": meta, "results": results }))
    }

    fn result_json(&self, s: &SessionSummary, hits: &SessionHits) -> Value {
        let count = |k: Kind| hits.counts.get(&k).map_or(Value::Null, |n| (*n).into());
        let snippets: Vec<Value> = hits
            .snippets
            .iter()
            .map(|h| {
                json!({
                    "kind": h.kind.as_str(),
                    "tool": h.tool,
                    "turn": h.turn,
                    "time": h.ts.and_then(rfc3339),
                    "offset": h.offset,
                    "terms": h.terms,
                    "text": h.text.render("«", "»"),
                })
            })
            .collect();
        json!({
            "session_id": s.id,
            "title": title(s),
            "first_prompt": json_prompt(Some(&s.first_prompt)),
            "project": project_name(s),
            "project_path": s.project_path,
            "cwd": s.cwd,
            "branch": s.git_branch.as_deref().filter(|b| !b.is_empty()),
            "started_at": s.started_at.and_then(rfc3339),
            "updated_at": rfc3339(s.updated_at),
            "turns": hits.turns,
            "cost_usd": s.cost_usd,
            "main_model": primary_model(&s.model_usage),
            "kind": SessionKind::of(s).as_str(),
            "favorite": s.favorite,
            "transcript": s.file_path.as_ref().filter(|_| !s.history_only()).map(|p| p.display().to_string()),
            "resume": resume(s, &self.claude).ok(),
            "hits": {
                "total": hits.total(),
                "prompt": count(Kind::Prompt),
                "reply": count(Kind::Reply),
                "thinking": count(Kind::Thinking),
                "tool_input": count(Kind::ToolInput),
                "tool_output": count(Kind::ToolOutput),
                "other": count(Kind::Other),
            },
            "term_hits": hits.term_hits,
            "snippets": snippets,
        })
    }
}

/// `reply · turn 6 · 11-13 08:06`, `tool-output Bash · turn 6 · 11-13 08:06`.
fn snippet_label(hit: &Hit) -> String {
    let mut label = hit.kind.as_str().to_string();
    if let Some(tool) = &hit.tool {
        label.push_str(&format!(" {tool}"));
    }
    if let Some(turn) = hit.turn {
        label.push_str(&format!(" · turn {turn}"));
    }
    if let Some(ts) = hit.ts {
        label.push_str(&format!(" · {}", fmt_time(Some(ts))));
    }
    label
}

/// The current directory, read only when a `--project` is relative to it: a search run from a
/// directory deleted since needs it for nothing else.
fn current_dir_for(projects: &[String]) -> Result<PathBuf> {
    let Some(relative) = projects.iter().find(|p| ProjectMatcher::needs_cwd(p)) else { return Ok(PathBuf::new()) };
    std::env::current_dir().map_err(|e| anyhow::anyhow!("--project {relative} is relative, and the current directory cannot be read: {e}"))
}

/// What an empty result could widen, in the order of the spec. A project that does not exist
/// leaves only that to drop: widening anything else finds nothing either.
fn hints(f: &Found) -> Vec<String> {
    if f.no_such_project {
        return vec!["without --project".into()];
    }
    let a = f.args;
    let mut out: Vec<&str> = Vec::new();
    if f.range.limited() {
        out.push("--days 0 for all time");
    }
    if f.kinds.kinds().len() < Kind::ALL.len() {
        out.push("--in all for every kind");
    }
    if f.query.len() > 1 {
        out.push("fewer terms");
    }
    if f.query.same_message {
        out.push("without --same-message");
    }
    for (given, hint) in [
        (!a.projects.is_empty(), "without --project"),
        (!a.tools.is_empty(), "without --tool"),
        (a.model.is_some(), "without --model"),
        (a.favorites, "without --favorites"),
    ] {
        if given {
            out.push(hint);
        }
    }
    if out.is_empty() {
        out.push("other ways of saying each TERM");
    }
    out.into_iter().map(String::from).collect()
}
