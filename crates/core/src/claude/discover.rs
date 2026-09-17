//! Find the top-level transcript files under `~/.claude/projects`.

use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::paths::ClaudeDir;
use crate::claude::parse::mtime_ms;

/// One `~/.claude/projects/<project>/<session>.jsonl` with its stat data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptFile {
    pub project_key: String,
    pub session_id: String,
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
    pub mtime_ns: i64,
    pub inode: u64,
}

/// Every top-level transcript, projects in name order. Subagent transcripts
/// (`<session>/subagents/*.jsonl`) are not sessions of their own and are left out.
/// Fails when the projects directory does not exist.
pub fn discover_transcripts(claude: &ClaudeDir) -> Result<Vec<TranscriptFile>> {
    let projects = claude.projects_dir();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&projects)
        .with_context(|| format!("projects dir not found: {}", projects.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    let mut out = Vec::new();
    for dir in dirs {
        let project_key = dir.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            out.push(TranscriptFile {
                project_key: project_key.clone(),
                session_id: path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string(),
                size: meta.len(),
                mtime_ms: mtime_ms(&meta),
                mtime_ns: meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
                inode: meta.ino(),
                path,
            });
        }
    }
    Ok(out)
}

/// Number of subagent transcripts of a session (`<project>/<session>/subagents/*.jsonl`).
pub fn subagent_count(transcript: &std::path::Path) -> u32 {
    let Some(stem) = transcript.file_stem() else { return 0 };
    let dir = transcript.with_file_name(stem).join("subagents");
    match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("jsonl"))
            .count() as u32,
        Err(_) => 0,
    }
}
