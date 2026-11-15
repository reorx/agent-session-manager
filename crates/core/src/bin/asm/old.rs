//! `list`, `session-old` and `search-old`: what `claude_session_search.py` prints, compared byte
//! for byte by `scripts/compare-with-python.sh`. Their output never changes; agents use the new
//! `search` / `session` instead.

use std::io::Write;

use agent_session_core::claude::paths::{ClaudeDir, encode_project_path, expand_user};
use agent_session_core::format::{fmt_datetime, fmt_time, human_size};
use agent_session_core::index::{Db, ListFilter};
use agent_session_core::model::{Role, SessionSummary};
use agent_session_core::search::fts::{FtsQuery, FtsRoles, search_sessions};
use agent_session_core::search::regex::{RegexOptions, RoleSet, Snippet, ToolLineKind, list_transcripts, search};
use anyhow::{Result, anyhow, bail};
use clap::{Args, ValueEnum};

use crate::App;

const PROMPT_LINE_LEN: usize = 240;

#[derive(Args)]
pub struct ListArgs {
    #[arg(long)]
    favorites: bool,
    /// Project directory, or a substring of project directory names (default: cwd)
    path: Option<String>,
    /// Only sessions active within the last DAYS days
    #[arg(short = 'd', long)]
    days: Option<f64>,
    #[arg(short = 'n', long, default_value_t = 200)]
    limit: usize,
    /// Print first prompts in full
    #[arg(long)]
    full: bool,
    /// Do not refresh the index first
    #[arg(long)]
    no_scan: bool,
}

#[derive(Args)]
pub struct SessionArgs {
    id: String,
    /// Read turn cards directly from the transcript, without accessing the index
    #[arg(long)]
    conversation: bool,
    /// Highlight a regular expression in the conversation
    #[arg(long = "match", requires = "conversation")]
    pattern: Option<String>,
    #[arg(short = 'n', long, default_value_t = 200)]
    limit: usize,
    #[arg(long)]
    no_scan: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RoleArg {
    User,
    Assistant,
    Tool,
}

#[derive(Args)]
pub struct SearchArgs {
    /// Regular expression (literal text when it does not compile)
    query: String,
    /// Only transcripts modified within the last DAYS days (0 = all). Default: 7
    #[arg(short = 'd', long)]
    days: Option<f64>,
    /// Only projects whose directory name contains this (case-insensitive)
    #[arg(short = 'p', long)]
    project: Option<String>,
    /// Message roles to search (repeatable). Default: all
    #[arg(short = 'r', long = "role")]
    roles: Vec<RoleArg>,
    /// Only pieces whose label contains this: thinking, tool_use:NAME, tool_result (repeatable)
    #[arg(short = 'l', long = "label")]
    labels: Vec<String>,
    #[arg(long)]
    case_sensitive: bool,
    /// Characters of context on each side of a match
    #[arg(short = 'C', long, default_value_t = 80)]
    context: usize,
    /// Stop after this many matches (sessions with --index)
    #[arg(short = 'n', long, default_value_t = 200)]
    limit: usize,
    /// Full-text search over the index instead of scanning transcripts
    #[arg(long)]
    index: bool,
    #[arg(long)]
    no_scan: bool,
}

impl App {
    pub(crate) fn session_old(&mut self, args: SessionArgs) -> Result<()> {
        if args.conversation { self.conversation(&args.id, args.pattern.as_deref()) }
        else { self.session(&args.id, args.limit, args.no_scan) }
    }

    fn conversation(&mut self, id: &str, pattern: Option<&str>) -> Result<()> {
        use agent_session_core::{claude::conversation::read_conversation, model::{ConvItem, StepKind, TurnEnd}, search::Matcher};
        let favorites = ClaudeDir::new(agent_session_core::config::favorites_dir(&self.db_path)?.join("claude"));
        let path = agent_session_core::transcript::resolve_transcript(&self.claude, Some(&favorites), id)?;
        let matcher = pattern.map(|p| Matcher::regex(p, true));
        let start = std::time::Instant::now();
        let conv = read_conversation(&path, matcher.as_ref())?;
        let elapsed = start.elapsed();
        let hits = conv.hit_places();
        let mut counts = vec![0; conv.items.len()];
        for hit in &hits { counts[hit.item] += 1; }
        let (mut turns, mut autos, mut compacts, mut tools) = (0, 0, 0, 0);
        let (mut interrupted, mut errors, mut completed, mut no_reply) = (0, 0, 0, 0);
        for (i, item) in conv.items.iter().enumerate() {
            match item {
                ConvItem::Compact(_) => {
                    compacts += 1;
                    writeln!(self.out, "COMPACT hits={}", counts[i])?;
                }
                ConvItem::Turn(t) => {
                    let label = if let Some(n) = t.number { turns += 1; format!("TURN {n}") }
                        else { autos += 1; format!("AUTO {}", t.origin.as_deref().unwrap_or("unknown")) };
                    tools += t.steps.iter().filter(|s| matches!(s.kind, StepKind::Tool(_))).count();
                    let end = match t.end {
                        TurnEnd::Completed => { completed += 1; "completed" }
                        TurnEnd::Interrupted { .. } => { interrupted += 1; "interrupted" }
                        TurnEnd::ApiError { .. } => { errors += 1; "api-error" }
                        TurnEnd::NoReply => { no_reply += 1; "no-reply" }
                    };
                    writeln!(self.out, "{label} steps={} end={end} hits={}", t.steps.len(), counts[i])?;
                }
            }
        }
        writeln!(self.out, "total turns={turns} auto={autos} compact={compacts} tools={tools} completed={completed} interrupted={interrupted} api-error={errors} no-reply={no_reply} hits={} skipped={} bytes={} read_ms={:.3}",
            hits.len(), conv.skipped_lines, conv.bytes, elapsed.as_secs_f64() * 1000.)?;
        Ok(())
    }

    /// Encoded project names matching `arg`: a directory, or a substring of a name.
    pub(crate) fn resolve_project_keys(&self, arg: &str, db: &Db) -> Result<Vec<String>> {
        let mut known = db.project_keys()?;
        // The reference also lists project directories containing only memory or metadata.
        let projects = self.claude.projects_dir();
        if projects.is_dir() {
            for entry in std::fs::read_dir(projects)? {
                let entry = entry?;
                if entry.path().is_dir() {
                    known.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
        known.sort();
        known.dedup();
        let p = expand_user(arg);
        if p.is_dir() {
            let resolved = p.canonicalize().unwrap_or(p);
            let key = encode_project_path(&resolved.to_string_lossy());
            if known.contains(&key) {
                return Ok(vec![key]);
            }
            bail!(
                "no sessions recorded for {} (neither {} nor {})",
                resolved.display(),
                self.claude.projects_dir().join(&key).display(),
                self.claude.history_file().display()
            );
        }
        let needle = arg.to_lowercase();
        let hits: Vec<String> = known.into_iter().filter(|k| k.to_lowercase().contains(&needle)).collect();
        if hits.is_empty() {
            bail!(
                "no project matches '{arg}' in {} or {}",
                self.claude.projects_dir().display(),
                self.claude.history_file().display()
            );
        }
        Ok(hits)
    }

    pub(crate) fn list(&mut self, args: ListArgs) -> Result<()> {
        let db = self.open_db(!args.no_scan)?;
        let arg = args.path.clone().unwrap_or_else(|| ".".to_string());
        let keys = self.resolve_project_keys(&arg, &db)?;
        let since_ms = args.days.filter(|d| *d > 0.0).map(|d| chrono::Utc::now().timestamp_millis() - (d * 86_400_000.0) as i64);
        let mut shown = 0;
        for key in keys {
            let infos = db.list_sessions(&ListFilter { favorites_only: args.favorites, project_keys: Some(vec![key.clone()]), since_ms, ..Default::default() })?;
            let n_hist = infos.iter().filter(|s| s.history_only()).count();
            let (bold, dim, reset) = (self.pal.bold, self.pal.dim, self.pal.reset);
            writeln!(self.out, "{bold}{key}{reset} {dim}({} sessions, {} with transcript){reset}", infos.len(), infos.len() - n_hist)?;
            for info in &infos {
                if shown >= args.limit {
                    writeln!(self.out, "\n{dim}… limit of {} sessions reached, use -n to show more{reset}", args.limit)?;
                    break;
                }
                self.print_session(info, args.full)?;
                shown += 1;
            }
            writeln!(self.out)?;
        }
        Ok(())
    }

    fn print_session(&mut self, info: &SessionSummary, full: bool) -> Result<()> {
        let p = &self.pal;
        let sid = if info.id_known { info.id.as_str() } else { "(unknown session id)" };
        let meta = match info.file_size.filter(|_| !info.history_only()) {
            Some(size) => format!("{} prompts, {}, updated {}", info.user_msg_count, human_size(size), fmt_datetime(Some(info.updated_at))),
            None => format!("{} prompts, updated {}", info.user_msg_count, fmt_datetime(Some(info.updated_at))),
        };
        let tag = if info.history_only() { format!(" {}[history only]{}", p.dim, p.reset) } else { String::new() };
        writeln!(self.out, "\n{}{}{}  {}{sid}{}  {}{meta}{}{tag}", p.blue, fmt_datetime(info.started_at), p.reset, p.bold, p.reset, p.gray, p.reset)?;
        let prompt = if info.first_prompt.is_empty() { format!("{}(no prompt){}", p.dim, p.reset) } else { info.first_prompt.clone() };
        if full {
            for line in prompt.lines() {
                writeln!(self.out, "    {line}")?;
            }
        } else {
            writeln!(self.out, "    {}", agent_session_core::claude::clean::one_line(&prompt, PROMPT_LINE_LEN))?;
        }
        Ok(())
    }

    fn session(&mut self, id: &str, limit: usize, no_scan: bool) -> Result<()> {
        let db = self.open_db(!no_scan)?;
        let found = db.find_by_prefix(id)?;
        if found.is_empty() {
            bail!(
                "no session matches '{id}' in {} or {} (a session that never received a prompt leaves no record)",
                self.claude.projects_dir().display(),
                self.claude.history_file().display()
            );
        }
        for info in found.iter().take(limit) {
            if info.favorite { writeln!(self.out, "favorite: {}", info.file_path.as_ref().unwrap().display())?; }
            writeln!(self.out, "{}{}{}", self.pal.bold, info.project_key, self.pal.reset)?;
            self.print_session(info, true)?;
            writeln!(self.out)?;
        }
        Ok(())
    }

    pub(crate) fn search_old(&mut self, args: SearchArgs) -> Result<()> {
        if args.index {
            return self.search_index(args);
        }
        let roles = if args.roles.is_empty() {
            RoleSet::all()
        } else {
            RoleSet {
                user: args.roles.contains(&RoleArg::User),
                assistant: args.roles.contains(&RoleArg::Assistant),
                tool: args.roles.contains(&RoleArg::Tool),
            }
        };
        let opts = RegexOptions { roles, labels: args.labels.clone(), case_sensitive: args.case_sensitive, context: args.context, limit: args.limit };
        let files = list_transcripts(&self.claude, Some(&ClaudeDir::new(agent_session_core::config::favorites_dir(&self.db_path)?.join("claude"))), Some(args.days.unwrap_or(7.0)), args.project.as_deref())?;
        if files.is_empty() {
            writeln!(self.out, "no sessions found in the given range")?;
            return Ok(());
        }
        let outcome = search(&args.query, &files, &opts);
        for fm in &outcome.files {
            let p = &self.pal;
            let name = fm.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            writeln!(self.out, "\n{}{}{} {}{name}  (updated {}, {} matches){}", p.bold, fm.project_key, p.reset, p.dim, fmt_datetime(Some(fm.mtime_ms)), fm.match_count, p.reset)?;
            for m in &fm.matches {
                let role = m.role.as_str();
                let tag = if m.detail.is_empty() { role.to_string() } else { format!("{role}/{}", m.detail) };
                let header = format!("\n{}{}{} {}[{tag}]{}", p.blue, fmt_time(m.timestamp_ms), p.reset, p.role(role), p.reset);
                match &m.snippet {
                    Snippet::ToolUse(lines) => {
                        writeln!(self.out, "{header}")?;
                        for line in lines {
                            match &line.kind {
                                ToolLineKind::Description => writeln!(self.out, "{}# {}{}", p.gray, p.highlight(&line.text, p.gray), p.reset)?,
                                ToolLineKind::Command => writeln!(self.out, "{}${} {}", p.gray, p.reset, p.highlight(&line.text, ""))?,
                                ToolLineKind::Field(k) => writeln!(self.out, "{}{k}:{} {}", p.dim, p.reset, p.highlight(&line.text, ""))?,
                            }
                        }
                    }
                    Snippet::Text(h) => {
                        let text = p.highlight(h, "");
                        if text.contains('\n') {
                            writeln!(self.out, "{header}")?;
                            for line in text.split('\n') {
                                writeln!(self.out, "      {line}")?;
                            }
                        } else {
                            writeln!(self.out, "{header} {text}")?;
                        }
                    }
                }
            }
        }
        let more = if outcome.total > args.limit { format!(" (showing first {})", args.limit) } else { String::new() };
        writeln!(self.out, "\n{}{} matches{} in {} sessions scanned{more}", self.pal.bold, outcome.total, self.pal.reset, outcome.scanned)?;
        Ok(())
    }

    fn search_index(&mut self, args: SearchArgs) -> Result<()> {
        if args.roles.contains(&RoleArg::Tool) {
            return Err(anyhow!("tool inputs and outputs are not indexed; drop --index to search them"));
        }
        let db = self.open_db(!args.no_scan)?;
        let roles = if args.roles.is_empty() {
            FtsRoles::both()
        } else {
            FtsRoles { user: args.roles.contains(&RoleArg::User), assistant: args.roles.contains(&RoleArg::Assistant) }
        };
        let since_ms = args.days.filter(|d| *d > 0.0).map(|d| chrono::Utc::now().timestamp_millis() - (d * 86_400_000.0) as i64);
        let started = std::time::Instant::now();
        let hits = search_sessions(&db, &FtsQuery { text: args.query.clone(), roles, project: args.project.clone(), since_ms, sessions: None, limit: args.limit })?;
        let elapsed = started.elapsed();
        let mut total = 0;
        for hit in &hits {
            total += hit.count as usize;
            let p = &self.pal;
            let updated = db.session(&hit.session_id)?.map(|s| s.updated_at).unwrap_or(hit.updated_at);
            writeln!(self.out, "\n{}{}{} {}{}  (updated {}, {} matches){}", p.bold, hit.project_key, p.reset, p.dim, hit.session_id, fmt_datetime(Some(updated)), hit.count, p.reset)?;
            let role = match hit.first_role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            writeln!(self.out, "\n{}{}{} {}[{role}]{} {}", p.blue, fmt_time(Some(hit.first_ts)), p.reset, p.role(role), p.reset, p.highlight(&hit.first, ""))?;
        }
        writeln!(self.out, "\n{}{total} matches{} in {} sessions ({:.1} ms)", self.pal.bold, self.pal.reset, hits.len(), elapsed.as_secs_f64() * 1000.0)?;
        Ok(())
    }
}
