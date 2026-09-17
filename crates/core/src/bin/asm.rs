//! `asm`: command-line front end of the core crate and the data layer's test bench.
//! `list` / `session` / `search` print what `claude_session_search.py` prints.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use agent_session_core::claude::paths::{ClaudeDir, default_db_path, encode_project_path, expand_user};
use agent_session_core::format::{fmt_datetime, fmt_time, human_size};
use agent_session_core::index::{Db, Indexer, ListFilter, Progress};
use agent_session_core::model::{Role, SessionSummary};
use agent_session_core::search::fts::{FtsQuery, FtsRoles, search_sessions};
use agent_session_core::search::regex::{RegexOptions, RoleSet, Snippet, ToolLineKind, list_transcripts, search};
use agent_session_core::search::Highlighted;
use anyhow::{Result, anyhow, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};

const PROMPT_LINE_LEN: usize = 240;

#[derive(Parser)]
#[command(name = "asm", version, about = "Index, list and search Claude Code sessions")]
struct Cli {
    /// Claude data directory (default: $CLAUDE_CONFIG_DIR or ~/.claude)
    #[arg(long, env = "ASM_CLAUDE_DIR", global = true)]
    claude_dir: Option<PathBuf>,
    /// SQLite index file (default: ~/Library/Application Support/AgentSessionManager/index.sqlite)
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
        #[arg(short = 'n', long, default_value_t = 200)]
        limit: usize,
        #[arg(long)]
        no_scan: bool,
    },
    /// Search message text (regex over transcripts; --index for full-text search over the index)
    Search(SearchArgs),
    /// Index statistics
    Stats,
}

#[derive(Args)]
struct ListArgs {
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
        Cmd::Index { rebuild } => app.index(rebuild),
        Cmd::List(args) => app.list(args),
        Cmd::Session { id, limit, no_scan } => app.session(&id, limit, no_scan),
        Cmd::Search(args) => app.search(args),
        Cmd::Stats => app.stats(),
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
    fn open_db(&self, scan: bool) -> Result<Db> {
        let mut db = Db::open(&self.db_path)?;
        if scan {
            Indexer::new(self.claude.clone()).scan(&mut db, &mut |_| {})?;
        }
        Ok(db)
    }

    fn index(&mut self, rebuild: bool) -> Result<()> {
        let mut db = Db::open(&self.db_path)?;
        let indexer = Indexer::new(self.claude.clone());
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
        let known = db.project_keys()?;
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
            let infos = db.list_sessions(&ListFilter { project_keys: Some(vec![key.clone()]), since_ms, ..Default::default() })?;
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
        let files = list_transcripts(&self.claude, Some(args.days.unwrap_or(7.0)), args.project.as_deref())?;
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
        let hits = search_sessions(&db, &FtsQuery { text: args.query.clone(), roles, project: args.project.clone(), since_ms, limit: args.limit })?;
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
