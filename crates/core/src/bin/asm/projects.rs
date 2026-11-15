//! `projects`: the projects that have sessions, most recently active first, for agents to find
//! the `--project` value when the user names an app or a repository loosely.

use std::collections::BTreeMap;
use std::io::Write;
use std::time::Instant;

use agent_session_core::format::fmt_datetime;
use agent_session_core::search::sessions::{SessionFilter, select_sessions};
use anyhow::Result;
use clap::Args;
use serde_json::{Value, json};

use crate::agent::{RangeArgs, excluded_session, now_ms, plural, rfc3339};
use crate::App;

#[derive(Args)]
pub struct ProjectsArgs {
    #[command(flatten)]
    range: RangeArgs,
    /// At most N projects; 0 for no limit
    #[arg(short = 'n', long, value_name = "N", default_value_t = 50)]
    limit: usize,
    /// Count the session asm runs inside (CLAUDE_CODE_SESSION_ID) too
    #[arg(long)]
    include_self: bool,
    /// JSON output
    #[arg(long)]
    json: bool,
    /// Do not refresh the index first
    #[arg(long)]
    no_scan: bool,
}

/// A project with sessions in range.
struct Project {
    key: String,
    path: Option<String>,
    sessions: usize,
    updated_at: i64,
}

impl Project {
    /// The path, else the encoded directory name: what the text line shows and sorts ties by.
    fn shown_path(&self) -> &str {
        self.path.as_deref().unwrap_or(&self.key)
    }

    fn name(&self) -> String {
        let path = self.shown_path().trim_end_matches('/');
        path.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or(&self.key).to_string()
    }
}

impl App {
    pub(crate) fn list_projects(&mut self, args: ProjectsArgs) -> Result<()> {
        let json = args.json;
        let result = self.list_projects_inner(&args);
        self.fail_as_json(json, result)
    }

    fn list_projects_inner(&mut self, args: &ProjectsArgs) -> Result<()> {
        let started = Instant::now();
        let range = args.range.resolve(now_ms(), None)?;
        let excluded = excluded_session(args.include_self);
        let (db, refresh) = self.open_db_refreshed(!args.no_scan)?;
        let filter = SessionFilter { since_ms: range.since_ms, until_ms: range.until_ms, exclude: excluded.clone(), ..SessionFilter::default() };
        let mut by_key: BTreeMap<String, Project> = BTreeMap::new();
        for s in select_sessions(&db, &filter)? {
            let p = by_key.entry(s.project_key.clone()).or_insert_with(|| Project { key: s.project_key.clone(), path: None, sessions: 0, updated_at: i64::MIN });
            p.sessions += 1;
            p.updated_at = p.updated_at.max(s.updated_at);
            if p.path.is_none() {
                p.path = s.project_path.clone();
            }
        }
        let mut projects: Vec<Project> = by_key.into_values().collect();
        projects.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.shown_path().cmp(b.shown_path())));
        let total = projects.len();
        if args.limit > 0 {
            projects.truncate(args.limit);
        }
        let shown = projects.len();
        let elapsed_ms = started.elapsed().as_millis();
        if args.json {
            let meta = json!({
                "command": "projects",
                "range": range.json(),
                "excluded_session": excluded,
                "index_refresh": refresh.as_str(),
                "elapsed_ms": elapsed_ms,
                "total": total,
                "shown": shown,
                "truncated": shown < total,
            });
            let results: Vec<Value> = projects
                .iter()
                .map(|p| {
                    json!({
                        "project_path": p.path,
                        "project": p.name(),
                        "project_key": p.key,
                        "sessions": p.sessions,
                        "updated_at": rfc3339(p.updated_at),
                    })
                })
                .collect();
            return self.write_json(&json!({ "meta": meta, "results": results }));
        }
        let mut head = plural(total, "project", "projects");
        if shown < total {
            head.push_str(&format!(" (showing {shown})"));
        }
        writeln!(self.out, "{head} · {} · {:.2}s", range.label, elapsed_ms as f64 / 1000.0)?;
        let width = projects.iter().map(|p| p.sessions.to_string().len()).max().unwrap_or(0).max(3);
        for p in &projects {
            let noun = if p.sessions == 1 { "session" } else { "sessions" };
            writeln!(self.out, "{}  {:>width$} {noun:<8}  {}", fmt_datetime(Some(p.updated_at)), p.sessions, p.shown_path())?;
        }
        if shown < total {
            writeln!(self.out, "showing {shown} of {total} projects, use -n for more")?;
        }
        Ok(())
    }
}
