//! Find the transcript files under `~/.claude/projects`: one per session, plus its subagents'.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::paths::ClaudeDir;
use crate::claude::parse::mtime_ms;

/// One `~/.claude/projects/<project>/<session>.jsonl` (or one of its subagents' transcripts)
/// with its stat data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptFile {
    pub favorite: bool,
    pub project_key: String,
    /// For a subagent transcript: the session it belongs to.
    pub session_id: String,
    /// Empty for a session's own transcript, else the subagent file stem (`agent-a1b2c3`).
    pub agent: String,
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
    pub mtime_ns: i64,
    pub inode: u64,
}

/// Every top-level transcript, projects in name order. Subagent transcripts
/// (`<session>/subagents/*.jsonl`) are not sessions of their own and are left out
/// (`discover_subagent_transcripts` lists them).
/// Fails when the projects directory does not exist.
pub fn discover_transcripts(claude: &ClaudeDir) -> Result<Vec<TranscriptFile>> {
    let mut out = Vec::new();
    for dir in read_dirs(&claude.projects_dir())? {
        let project_key = dir.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let entries = read_entries(&dir)?;
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let meta = match entry.metadata() { Ok(m) => m, Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue, Err(e) => return Err(e.into()) };
            if !meta.is_file() {
                continue;
            }
            let session_id = stem(&path);
            out.push(stat_file(&project_key, session_id, String::new(), path, &meta));
        }
    }
    Ok(out)
}

/// Every subagent transcript (`<project>/<session>/subagents/*.jsonl`), tagged with the session
/// it belongs to. Their API calls count towards that session's cost.
pub fn discover_subagent_transcripts(claude: &ClaudeDir) -> Result<Vec<TranscriptFile>> {
    let projects = claude.projects_dir();
    let mut out = Vec::new();
    for project in read_dirs(&projects)? {
        let project_key = project.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        for session in read_dirs(&project)? {
            let session_id = session.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
            let entries = read_entries(&session.join("subagents"))?;
            for entry in entries {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                let meta = match entry.metadata() { Ok(m) => m, Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue, Err(e) => return Err(e.into()) };
                if meta.is_file() {
                    out.push(stat_file(&project_key, session_id.clone(), stem(&path), path, &meta));
                }
            }
        }
    }
    Ok(out)
}

fn read_entries(dir: &Path) -> Result<Vec<std::fs::DirEntry>> {
    match std::fs::read_dir(dir) {
        Ok(entries) => Ok(entries.collect::<std::io::Result<Vec<_>>>()?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}
fn read_dirs(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in read_entries(dir)? {
        match entry.file_type() {
            Ok(t) if t.is_dir() => dirs.push(entry.path()),
            Ok(_) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.into()),
        }
    }
    dirs.sort(); Ok(dirs)
}

pub struct Discovered { pub sessions: Vec<TranscriptFile>, pub subagents: Vec<TranscriptFile> }
pub fn discover_all(claude: &ClaudeDir, favorites: Option<&ClaudeDir>) -> Result<Discovered> {
    let mut sessions = Vec::new(); let mut subagents = Vec::new();
    if let Some(favorites) = favorites {
        sessions = discover_transcripts(favorites)?;
        for file in &mut sessions { file.favorite = true; }
        subagents = discover_subagent_transcripts(favorites)?;
    }
    let ids: std::collections::HashSet<_> = sessions.iter().map(|f| f.session_id.clone()).collect();
    subagents.retain(|f| ids.contains(&f.session_id));
    for file in &mut subagents { file.favorite = true; }
    sessions.extend(discover_transcripts(claude)?.into_iter().filter(|f| !ids.contains(&f.session_id)));
    subagents.extend(discover_subagent_transcripts(claude)?.into_iter().filter(|f| !ids.contains(&f.session_id)));
    Ok(Discovered { sessions, subagents })
}

fn stem(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()
}

fn stat_file(project_key: &str, session_id: String, agent: String, path: PathBuf, meta: &std::fs::Metadata) -> TranscriptFile {
    TranscriptFile {
        favorite: false,
        project_key: project_key.to_string(),
        session_id,
        agent,
        size: meta.len(),
        mtime_ms: mtime_ms(meta),
        mtime_ns: meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
        inode: meta.ino(),
        path,
    }
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
