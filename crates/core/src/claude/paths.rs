//! Locations under `~/.claude` and the project-path encoding Claude Code uses for directory names.

use std::path::{Path, PathBuf};

/// The Claude data directory (default `~/.claude`).
#[derive(Clone, Debug)]
pub struct ClaudeDir {
    root: PathBuf,
}

impl ClaudeDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ClaudeDir { root: root.into() }
    }

    /// `$ASM_CLAUDE_DIR`, else `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
    pub fn from_env() -> Self {
        for var in ["ASM_CLAUDE_DIR", "CLAUDE_CONFIG_DIR"] {
            if let Some(dir) = std::env::var_os(var).filter(|v| !v.is_empty()) {
                return ClaudeDir::new(dir);
            }
        }
        ClaudeDir::new(home_dir().join(".claude"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.root.join("projects")
    }

    pub fn history_file(&self) -> PathBuf {
        self.root.join("history.jsonl")
    }

    pub fn paste_cache_dir(&self) -> PathBuf {
        self.root.join("paste-cache")
    }
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Expand a leading `~` like the shell would.
pub fn expand_user(path: &str) -> PathBuf {
    if path == "~" {
        home_dir()
    } else if let Some(rest) = path.strip_prefix("~/") {
        home_dir().join(rest)
    } else {
        PathBuf::from(path)
    }
}

/// `/Users/me/Code/campus_watch` → `-Users-me-Code-campus-watch`: every byte that is not a
/// letter, digit or `-` becomes `-` (irreversible, so real paths come from `cwd`).
pub fn encode_project_path(path: &str) -> String {
    path.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect()
}

/// Where the SQLite index lives by default.
pub fn default_db_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ASM_DB").filter(|v| !v.is_empty()) {
        return PathBuf::from(p);
    }
    match directories::ProjectDirs::from("dev", "reorx", "AgentSessionManager") {
        Some(dirs) => dirs.data_dir().join("index.sqlite"),
        None => home_dir().join(".agent-session-manager").join("index.sqlite"),
    }
}
