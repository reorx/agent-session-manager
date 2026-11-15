//! `index`, `stats` and the favorites commands: keeping the index and the favorite copies.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use agent_session_core::claude::paths::ClaudeDir;
use agent_session_core::format::human_size;
use agent_session_core::index::{Indexer, Progress};
use anyhow::{Result, anyhow, bail};

use crate::App;

impl App {
    pub(crate) fn favorite(&mut self, id: &str, remove: bool, yes: bool) -> Result<()> {
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
        self.refresh_with(&Indexer::new(self.claude.clone(), Some(ClaudeDir::new(root.join("claude")))), &mut db)?;
        writeln!(self.out, "{} {}", if remove { "Unfavorited" } else { "Favorited" }, s.id)?; Ok(())
    }

    pub(crate) fn favorites_dir(&mut self, path: Option<PathBuf>, reset: bool) -> Result<()> {
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
        self.refresh_with(&Indexer::new(self.claude.clone(), Some(ClaudeDir::new(new.join("claude")))), &mut db)?;
        writeln!(self.out, "{} (moved {}, skipped {})", new.display(), report.moved, report.skipped)?; Ok(())
    }

    /// Unlike the other commands, `index` fails when the app holds the write lock: writing the
    /// index is all it is for.
    pub(crate) fn index(&mut self, rebuild: bool) -> Result<()> {
        let mut db = self.open_db(false)?;
        let indexer = self.indexer()?;
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
        let stale = match report.files_stale {
            0 => String::new(),
            n => format!(", {n} indexed meanwhile by another process"),
        };
        writeln!(
            self.out,
            "{} files ({} new, {} appended, {} rebuilt, {} unchanged, {} missing{}), {} history sessions, {} parsed in {:.3}s (parse {:.3}s, write {:.3}s, history {:.3}s)",
            report.files_seen,
            report.files_parsed,
            report.files_appended,
            report.files_rebuilt,
            report.files_unchanged,
            report.files_missing,
            stale,
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

    pub(crate) fn stats(&mut self) -> Result<()> {
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
}
