//! `asm`: command-line front end of the core crate and the data layer's test bench. The
//! specification is `skills/asm/references/cli.md` (`spec.rs` checks the two agree).
//! `list` / `session-old` / `search-old` print what `claude_session_search.py` prints.

use std::ffi::OsStr;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use agent_session_core::claude::paths::{ClaudeDir, default_db_path};
use agent_session_core::index::{Db, Indexer, is_busy};
use agent_session_core::search::Highlighted;
use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

mod cost;
mod manage;
mod old;
#[cfg(test)]
mod spec;
#[path = "../support/transcript.rs"]
mod transcript;

/// How long a refresh waits for the app to release the index's write lock before giving up.
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a refresh waits instead when no scan has gone through on the index yet, so another
/// process holding the lock is building it: a cold build of real data writes for 8-12 s
/// (2026-11), and agents' shell tools commonly give up on a command after two minutes.
const BUILD_TIMEOUT: Duration = Duration::from_secs(60);
const BUSY_NOTICE: &str = "index busy, results may miss the last few minutes";

#[derive(Parser)]
#[command(name = "asm", version, about = "Index, list and search Claude Code sessions")]
struct Cli {
    /// Claude data directory (default: $CLAUDE_CONFIG_DIR or ~/.claude)
    #[arg(long, env = "ASM_CLAUDE_DIR", global = true)]
    claude_dir: Option<PathBuf>,
    /// SQLite index file (default: ~/Library/Application Support/dev.reorx.AgentSessionManager/index.sqlite)
    #[arg(long, env = "ASM_DB", global = true)]
    db: Option<PathBuf>,
    /// When to print ANSI colors; auto: only when stdout is a terminal and NO_COLOR is unset or empty
    #[arg(long, value_enum, value_name = "WHEN", default_value_t = ColorWhen::Auto, global = true)]
    color: ColorWhen,
    /// Same as --color never, and wins over --color
    #[arg(long, global = true)]
    no_color: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum ColorWhen {
    Auto,
    Always,
    Never,
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
    List(old::ListArgs),
    /// Show the session whose id starts with ID (the reference script's output)
    #[command(name = "session-old")]
    SessionOld(old::SessionArgs),
    /// Search message text (regex over transcripts; --index for full-text search over the index;
    /// the reference script's output)
    #[command(name = "search-old")]
    SearchOld(old::SearchArgs),
    /// What sessions cost (priced from token usage, subagents included) and how full their context got
    Cost(cost::CostArgs),
    /// Index statistics
    Stats,
}

/// `--no-color` is `--color never` and wins over any `--color`, wherever each is written (clap
/// cannot tell which came last once one is before the command name and one after); `always`
/// wins over `NO_COLOR`, as the convention asks of explicit flags.
fn use_color(when: ColorWhen, no_color: bool, stdout_tty: bool, no_color_env: Option<&OsStr>) -> bool {
    match when {
        _ if no_color => false,
        ColorWhen::Always => true,
        ColorWhen::Never => false,
        ColorWhen::Auto => stdout_tty && no_color_env.is_none_or(|v| v.is_empty()),
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
    let color = use_color(cli.color, cli.no_color, std::io::stdout().is_terminal(), std::env::var_os("NO_COLOR").as_deref());
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
        Cmd::SessionOld(args) => app.session_old(args),
        Cmd::SearchOld(args) => app.search_old(args),
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
    fn indexer(&self) -> Result<Indexer> {
        let favorites = agent_session_core::config::favorites_dir(&self.db_path)?.join("claude");
        Ok(Indexer::new(self.claude.clone(), Some(ClaudeDir::new(favorites))))
    }

    /// The index, first brought up to date with the files on disk when `scan` is set. An index
    /// that needs building while another process writes it cannot be used at all: that is an
    /// error, which says why it needs building. `main` prints only the outermost message, so any
    /// other failure to open is printed with its causes (a read-only file, a full disk).
    fn open_db(&self, scan: bool) -> Result<Db> {
        let mut db = Db::open_with(&self.db_path, BUSY_TIMEOUT).map_err(|e| match is_busy(&e) {
            true => {
                let why = e.to_string();
                e.context(format!("{why} (the app, or another asm); wait for it, quit it, or use another index with --db / ASM_DB"))
            }
            false => anyhow::anyhow!("{e:#}"),
        })?;
        if scan {
            self.refresh_with(&self.indexer()?, &mut db)?;
        }
        Ok(db)
    }

    fn refresh_with(&self, indexer: &Indexer, db: &mut Db) -> Result<()> {
        refresh(indexer, db, &self.db_path, BUILD_TIMEOUT)
    }
}

/// Scan with `indexer`. When another process (the app, or another `asm`) holds the write lock,
/// the scan is dropped (its transaction rolls back) and the command answers from the index as it
/// is. Unless no scan has gone through on the index yet (a new index, or one being rebuilt):
/// that process is building it, and answering now would answer from nothing, so the scan waits
/// for the lock up to `build_timeout` (it then finds the build's files written and skips them),
/// and fails if the build takes longer.
fn refresh(indexer: &Indexer, db: &mut Db, db_path: &Path, build_timeout: Duration) -> Result<()> {
    let scanned = Indexer::has_scanned(db)?;
    if !scanned {
        db.conn().busy_timeout(build_timeout)?;
    }
    let result = indexer.scan(db, &mut |_| {});
    db.conn().busy_timeout(BUSY_TIMEOUT)?;
    match result {
        Ok(_) => Ok(()),
        Err(e) if is_busy(&e) && scanned => {
            eprintln!("{BUSY_NOTICE}");
            Ok(())
        }
        Err(e) if is_busy(&e) => Err(e.context(format!(
            "{} is being built by another process (the app, or another asm), which still held it after {} s; run again when it is done, or use another index with --db / ASM_DB",
            db_path.display(),
            build_timeout.as_secs_f64()
        ))),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_go_to_a_terminal_unless_no_color_is_set_and_never_into_a_pipe() {
        let set = Some(OsStr::new("1"));
        let empty = Some(OsStr::new(""));
        assert!(use_color(ColorWhen::Auto, false, true, None));
        assert!(use_color(ColorWhen::Auto, false, true, empty), "an empty NO_COLOR is unset");
        assert!(!use_color(ColorWhen::Auto, false, true, set));
        assert!(!use_color(ColorWhen::Auto, false, false, None), "a pipe");
        assert!(use_color(ColorWhen::Always, false, false, set), "always wins over NO_COLOR and a pipe");
        assert!(!use_color(ColorWhen::Never, false, true, None));
        assert!(!use_color(ColorWhen::Always, true, true, None), "--no-color");
    }

    /// A build that keeps the lock past the wait is an error, not an answer from an empty index.
    /// (Waiting for a build that ends in time is tested in `tests/cli.rs`.)
    #[test]
    fn a_build_that_holds_the_lock_past_the_wait_fails_the_command() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("claude");
        std::fs::create_dir_all(claude.join("projects")).unwrap();
        let indexer = Indexer::new(ClaudeDir::new(claude), None);
        let path = tmp.path().join("index.sqlite");
        let mut db = Db::open_with(&path, BUSY_TIMEOUT).unwrap();
        let builder = rusqlite::Connection::open(&path).unwrap();
        builder.execute_batch("BEGIN IMMEDIATE").unwrap();

        let wait = Duration::from_millis(300);
        let started = std::time::Instant::now();
        let e = refresh(&indexer, &mut db, &path, wait).unwrap_err();
        assert!(started.elapsed() >= wait, "waited for the build first: {:?}", started.elapsed());
        assert_eq!(
            e.to_string(),
            format!("{} is being built by another process (the app, or another asm), which still held it after 0.3 s; run again when it is done, or use another index with --db / ASM_DB", path.display())
        );
        assert!(!Indexer::has_scanned(&db).unwrap());

        builder.execute_batch("COMMIT").unwrap();
        refresh(&indexer, &mut db, &path, wait).unwrap();
        assert!(Indexer::has_scanned(&db).unwrap(), "the next command builds it");
    }
}
