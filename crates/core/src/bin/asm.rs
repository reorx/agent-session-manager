//! `asm`: command-line front end of the core crate and the data layer's test bench.
//! `list` / `session` / `search` print what `claude_session_search.py` prints.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use agent_session_core::claude::paths::{ClaudeDir, default_db_path, encode_project_path, expand_user};
use agent_session_core::format::{fmt_datetime, fmt_time, fmt_tokens, fmt_usd, human_size};
use agent_session_core::index::{Db, Indexer, ListFilter, Progress};
use agent_session_core::model::{Role, SessionSummary};
use agent_session_core::search::fts::{FtsQuery, FtsRoles, search_sessions};
use agent_session_core::search::regex::{RegexOptions, RoleSet, Snippet, ToolLineKind, list_transcripts, search};
use agent_session_core::search::Highlighted;
use anyhow::{Result, anyhow, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};

const PROMPT_LINE_LEN: usize = 240;

#[path = "support/transcript.rs"]
mod transcript;

#[derive(Parser)]
#[command(name = "asm", version, about = "Index, list and search Claude Code sessions")]
struct Cli {
    /// Claude data directory (default: $CLAUDE_CONFIG_DIR or ~/.claude)
    #[arg(long, env = "ASM_CLAUDE_DIR", global = true)]
    claude_dir: Option<PathBuf>,
    /// SQLite index file (default: ~/Library/Application Support/dev.reorx.AgentSessionManager/index.sqlite)
    #[arg(long, env = "ASM_DB", global = true)]
    db: Option<PathBuf>,
    /// Never print ANSI colors (also: NO_COLOR=1)
    #[arg(long, global = true)]
    no_color: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Favorite { id: String },
    Unfavorite { id: String, #[arg(long)] yes: bool },
    FavoritesDir { path: Option<PathBuf>, #[arg(long, conflicts_with = "path")] reset: bool },
    /// Export a transcript as compact text with XML-style tags for agent handoffs
    Transcript(transcript::ReaderArgs),
    /// Bring the index up to date (or rebuild it from scratch)
    Index {
        #[arg(long)]
        rebuild: bool,
    },
    /// List the sessions of a project, newest first
    List(ListArgs),
    /// Show the session whose id starts with ID
    Session {
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
    },
    /// Search message text (regex over transcripts; --index for full-text search over the index)
    Search(SearchArgs),
    /// What sessions cost (priced from token usage, subagents included) and how full their context got
    Cost(CostArgs),
    /// Index statistics
    Stats,
}

#[derive(Args)]
struct ListArgs {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RoleArg {
    User,
    Assistant,
    Tool,
}

#[derive(Args)]
struct SearchArgs {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum CostSort {
    Cost,
    Context,
}

#[derive(Args)]
struct CostArgs {
    /// Project directory, or a substring of project directory names (default: all projects)
    #[arg(short = 'p', long)]
    project: Option<String>,
    /// Only sessions updated on or after this local date (YYYY-MM-DD)
    #[arg(long)]
    since: Option<chrono::NaiveDate>,
    /// Only sessions updated on or before this local date (YYYY-MM-DD)
    #[arg(long)]
    until: Option<chrono::NaiveDate>,
    #[arg(long, value_enum, default_value_t = CostSort::Cost)]
    sort: CostSort,
    #[arg(short = 'n', long, default_value_t = 30)]
    limit: usize,
    /// Compare with the cost Claude Code recorded itself (`cost-state`) instead of listing
    #[arg(long)]
    compare: bool,
    #[arg(long)]
    no_scan: bool,
}

const COST_TITLE_LEN: usize = 60;

/// Local midnight of `date`, unix ms.
fn local_midnight(date: chrono::NaiveDate) -> Option<i64> {
    use chrono::TimeZone;
    chrono::Local.from_local_datetime(&date.and_hms_opt(0, 0, 0)?).earliest().map(|dt| dt.timestamp_millis())
}

fn project_name(s: &SessionSummary) -> &str {
    let path = s.project_path.as_deref().or(s.cwd.as_deref()).unwrap_or(&s.project_key);
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

/// `Iterator::sum` of no floats is `-0.0`, which prints as `$-0.00`.
fn sum(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(0.0, |a, b| a + b)
}

fn context_cell(s: &SessionSummary) -> String {
    match s.context_ratio() {
        Some(r) if r < 0.01 => "<1%".into(),
        Some(r) => format!("{:.0}%", r * 100.0),
        None => "—".into(),
    }
}

struct Palette {
    reset: &'static str,
    bold: &'static str,
    dim: &'static str,
    hl: &'static str,
    blue: &'static str,
    gray: &'static str,
}

impl Palette {
    fn new(color: bool) -> Self {
        if color {
            Palette { reset: "\x1b[0m", bold: "\x1b[1m", dim: "\x1b[2m", hl: "\x1b[1;31m", blue: "\x1b[34m", gray: "\x1b[38;5;245m" }
        } else {
            Palette { reset: "", bold: "", dim: "", hl: "", blue: "", gray: "" }
        }
    }

    fn role(&self, role: &str) -> &'static str {
        if self.reset.is_empty() {
            return "";
        }
        match role {
            "user" => "\x1b[36m",
            "assistant" => "\x1b[32m",
            _ => "\x1b[33m",
        }
    }

    fn highlight(&self, h: &Highlighted, resume: &str) -> String {
        h.render(self.hl, &format!("{}{}", self.reset, resume))
    }
}

struct App {
    claude: ClaudeDir,
    db_path: PathBuf,
    pal: Palette,
    out: std::io::BufWriter<std::io::Stdout>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let color = !cli.no_color && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let claude = match &cli.claude_dir {
        Some(dir) => ClaudeDir::new(dir),
        None => ClaudeDir::from_env(),
    };
    let db_path = cli.db.clone().unwrap_or_else(default_db_path);
    let mut app = App { claude, db_path, pal: Palette::new(color), out: std::io::BufWriter::new(std::io::stdout()) };
    let result = match cli.cmd {
        Cmd::Transcript(args) => transcript::run(args, &app.claude, &app.db_path, &mut app.out),
        Cmd::Index { rebuild } => app.index(rebuild),
        Cmd::List(args) => app.list(args),
        Cmd::Session { id, limit, no_scan, conversation, pattern } => {
            if conversation { app.conversation(&id, pattern.as_deref()) }
            else { app.session(&id, limit, no_scan) }
        },
        Cmd::Search(args) => app.search(args),
        Cmd::Cost(args) => app.cost(args),
        Cmd::Stats => app.stats(),
        Cmd::Favorite { id } => app.favorite(&id, false, false),
        Cmd::Unfavorite { id, yes } => app.favorite(&id, true, yes),
        Cmd::FavoritesDir { path, reset } => app.favorites_dir(path, reset),
    };
    let _ = app.out.flush();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

impl App {
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

    fn favorite(&mut self, id: &str, remove: bool, yes: bool) -> Result<()> {
        use agent_session_core::{config, favorites};
        let mut db = self.open_db(true)?;
        let found = db.find_by_prefix(id)?;
        let [s] = found.as_slice() else { bail!("session ID must match exactly one session"); };
        let root = config::favorites_dir(&self.db_path)?;
        if remove {
            if !yes && !favorites::original_exists(&self.claude, &s.project_key, &s.id) { bail!("original is gone; use --yes to trash the last copy"); }
            favorites::remove(&root, &s.project_key, &s.id, &favorites::Trash::from_env())?;
        } else {
            if s.history_only() { bail!("cannot favorite a history-only session"); }
            favorites::save(&root, s.file_path.as_ref().ok_or_else(|| anyhow!("no transcript"))?, &s.project_key, &s.id)?;
        }
        Indexer::new(self.claude.clone(), Some(ClaudeDir::new(root.join("claude")))).scan(&mut db, &mut |_| {})?;
        writeln!(self.out, "{} {}", if remove { "Unfavorited" } else { "Favorited" }, s.id)?; Ok(())
    }
    fn favorites_dir(&mut self, path: Option<PathBuf>, reset: bool) -> Result<()> {
        use agent_session_core::{config, favorites};
        let old = config::favorites_dir(&self.db_path)?;
        if path.is_none() && !reset { writeln!(self.out, "{}", old.display())?; return Ok(()); }
        anyhow::ensure!(!config::environment_override(), "Favorites directory is controlled by ASM_FAVORITES_DIR");
        let new = config::canonical_root(&path.clone().unwrap_or_else(|| config::default_favorites_dir(&self.db_path)))?;
        let mut db = self.open_db(true)?;
        let report = favorites::migrate_with_commit(&old, &new, &self.claude, || db.transaction(|db| {
            db.set_meta("favorites_root", &config::canonical_root(&new.join("claude"))?.to_string_lossy())?;
            config::set_favorites_dir(&self.db_path, if reset { None } else { Some(&new) })
        }))?;
        Indexer::new(self.claude.clone(), Some(ClaudeDir::new(new.join("claude")))).scan(&mut db, &mut |_| {})?;
        writeln!(self.out, "{} (moved {}, skipped {})", new.display(), report.moved, report.skipped)?; Ok(())
    }

    fn open_db(&self, scan: bool) -> Result<Db> {
        let mut db = Db::open(&self.db_path)?;
        if scan {
            Indexer::new(self.claude.clone(), Some(ClaudeDir::new(agent_session_core::config::favorites_dir(&self.db_path)?.join("claude")))).scan(&mut db, &mut |_| {})?;
        }
        Ok(db)
    }

    fn index(&mut self, rebuild: bool) -> Result<()> {
        let mut db = Db::open(&self.db_path)?;
        let indexer = Indexer::new(self.claude.clone(), Some(ClaudeDir::new(agent_session_core::config::favorites_dir(&self.db_path)?.join("claude"))));
        let stderr_tty = std::io::stderr().is_terminal();
        let mut progress = |p: &Progress| {
            if stderr_tty {
                eprint!("\r{} {}/{}", p.phase, p.done, p.total);
                if p.phase == "done" {
                    eprintln!();
                }
            }
        };
        let report = if rebuild { indexer.rebuild(&mut db, &mut progress)? } else { indexer.scan(&mut db, &mut progress)? };
        let stats = db.stats()?;
        writeln!(
            self.out,
            "{} files ({} new, {} appended, {} rebuilt, {} unchanged, {} missing), {} history sessions, {} parsed in {:.3}s (parse {:.3}s, write {:.3}s, history {:.3}s)",
            report.files_seen,
            report.files_parsed,
            report.files_appended,
            report.files_rebuilt,
            report.files_unchanged,
            report.files_missing,
            report.history_sessions,
            human_size(report.bytes_parsed),
            report.elapsed.as_secs_f64(),
            report.elapsed_parse.as_secs_f64(),
            report.elapsed_write.as_secs_f64(),
            report.elapsed_history.as_secs_f64()
        )?;
        writeln!(self.out, "index: {} sessions, {} messages, {} projects, {} ({})", stats.sessions, stats.messages, stats.projects, human_size(stats.db_bytes), self.db_path.display())?;
        Ok(())
    }

    fn stats(&mut self) -> Result<()> {
        let db = self.open_db(false)?;
        let s = db.stats()?;
        writeln!(self.out, "db: {}", self.db_path.display())?;
        writeln!(self.out, "sessions: {}", s.sessions)?;
        writeln!(self.out, "history only: {}", s.history_only)?;
        writeln!(self.out, "messages: {}", s.messages)?;
        writeln!(self.out, "projects: {}", s.projects)?;
        writeln!(self.out, "files: {}", s.files)?;
        writeln!(self.out, "size: {}", human_size(s.db_bytes))?;
        Ok(())
    }

    /// Encoded project names matching `arg`: a directory, or a substring of a name.
    fn resolve_project_keys(&self, arg: &str, db: &Db) -> Result<Vec<String>> {
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

    fn list(&mut self, args: ListArgs) -> Result<()> {
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

    fn search(&mut self, args: SearchArgs) -> Result<()> {
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

    fn cost(&mut self, args: CostArgs) -> Result<()> {
        let db = self.open_db(!args.no_scan)?;
        let project_keys = match &args.project {
            Some(arg) => Some(self.resolve_project_keys(arg, &db)?),
            None => None,
        };
        let filter = ListFilter {
            project_keys,
            since_ms: args.since.and_then(local_midnight),
            until_ms: args.until.and_then(|d| d.succ_opt()).and_then(local_midnight),
            ..Default::default()
        };
        let mut sessions = db.list_sessions(&filter)?;
        if args.compare {
            return self.compare_costs(&sessions, args.limit);
        }
        let key = |s: &SessionSummary| match args.sort {
            CostSort::Cost => s.cost_usd,
            CostSort::Context => s.context_ratio(),
        };
        sessions.sort_by(|a, b| {
            key(b).unwrap_or(f64::NEG_INFINITY).total_cmp(&key(a).unwrap_or(f64::NEG_INFINITY)).then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        let (bold, dim, reset) = (self.pal.bold, self.pal.dim, self.pal.reset);
        writeln!(self.out, "{bold}{:>9} {:>9} {:>5}  {:<16}  {:<8}  {:<16}  TITLE{reset}", "COST", "CC", "CTX", "UPDATED", "SESSION", "PROJECT")?;
        let usd = |c: Option<f64>| c.map_or_else(|| "—".to_string(), fmt_usd);
        for s in sessions.iter().take(args.limit) {
            let id: String = if s.id_known { s.id.chars().take(8).collect() } else { "(none)".into() };
            let title = agent_session_core::claude::clean::one_line(s.display_title(), COST_TITLE_LEN);
            writeln!(
                self.out,
                "{:>9} {dim}{:>9}{reset} {:>5}  {}  {id:<8}  {:<16}  {title}",
                usd(s.cost_usd),
                usd(s.total_cost_usd),
                context_cell(s),
                fmt_datetime(Some(s.updated_at)),
                project_name(s),
            )?;
        }
        let total = sum(sessions.iter().filter_map(|s| s.cost_usd));
        let cc = sum(sessions.iter().filter_map(|s| s.total_cost_usd));
        let shown = if sessions.len() > args.limit { format!(" (top {} shown)", args.limit) } else { String::new() };
        writeln!(self.out, "\n{bold}{} sessions · {}{reset} {dim}(CC {}){reset}{shown}", sessions.len(), fmt_usd(total), fmt_usd(cc))?;
        let tokens = |f: fn(&SessionSummary) -> u64| fmt_tokens(sessions.iter().map(f).sum());
        writeln!(
            self.out,
            "{dim}tokens: in {} · out {} · cache read {} · cache write {}{reset}",
            tokens(|s| s.tokens_in),
            tokens(|s| s.tokens_out),
            tokens(|s| s.tokens_cache_read),
            tokens(|s| s.tokens_cache_write)
        )?;
        let unpriced: std::collections::BTreeSet<&str> = sessions.iter().flat_map(|s| s.unpriced_models.iter().map(String::as_str)).collect();
        if !unpriced.is_empty() {
            writeln!(self.out, "{dim}not priced (unknown models): {}{reset}", unpriced.into_iter().collect::<Vec<_>>().join(", "))?;
        }
        Ok(())
    }

    /// How far the priced cost is from what Claude Code recorded, over the sessions where it
    /// recorded something.
    fn compare_costs(&mut self, sessions: &[SessionSummary], limit: usize) -> Result<()> {
        let pairs: Vec<(&SessionSummary, f64, f64)> =
            sessions.iter().filter_map(|s| Some((s, s.cost_usd.unwrap_or(0.0), s.total_cost_usd.filter(|c| *c > 0.0)?))).collect();
        let n = pairs.len();
        let off = |ours: f64, cc: f64| (ours - cc) / cc;
        let bucket = |lo: f64, hi: f64| pairs.iter().filter(|(_, o, c)| (lo..hi).contains(&off(*o, *c).abs())).count();
        let lower = pairs.iter().filter(|(_, o, c)| off(*o, *c) <= -0.10).count();
        let higher = pairs.iter().filter(|(_, o, c)| off(*o, *c) >= 0.10).count();
        let pct = |k: usize| if n == 0 { 0.0 } else { k as f64 * 100.0 / n as f64 };
        writeln!(self.out, "sessions with a cost recorded by Claude Code: {n}")?;
        for (label, k) in [("within 1%", bucket(0.0, 0.01)), ("1–10% off", bucket(0.01, 0.10)), (">10% lower", lower), (">10% higher", higher)] {
            writeln!(self.out, "  {label:<12} {k:>6}  ({:.1}%)", pct(k))?;
        }
        let ours = sum(pairs.iter().map(|(_, o, _)| *o));
        let cc = sum(pairs.iter().map(|(_, _, c)| *c));
        let diff = if cc > 0.0 { (ours - cc) / cc * 100.0 } else { 0.0 };
        writeln!(self.out, "total: {} priced here vs {} recorded ({diff:+.2}%)", fmt_usd(ours), fmt_usd(cc))?;
        let all = sum(sessions.iter().filter_map(|s| s.cost_usd));
        writeln!(self.out, "all {} sessions in range: {} priced here", sessions.len(), fmt_usd(all))?;
        let mut worst = pairs.clone();
        worst.sort_by(|a, b| (b.1 - b.2).abs().total_cmp(&(a.1 - a.2).abs()));
        if !worst.is_empty() && limit > 0 {
            writeln!(self.out, "\nlargest differences:")?;
        }
        for (s, o, c) in worst.iter().take(limit) {
            writeln!(self.out, "  {:>9} vs {:>9}  {:+6.1}%  {}  {}", fmt_usd(*o), fmt_usd(*c), off(*o, *c) * 100.0, &s.id[..s.id.len().min(8)], project_name(s))?;
        }
        Ok(())
    }
}
