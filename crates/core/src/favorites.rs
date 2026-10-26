//! Durable session snapshots. The main JSONL is the commit marker for discovery.
use std::{fs::{self, File}, path::{Path, PathBuf}, io::{Read, Seek, SeekFrom}};
use anyhow::{Result, Context, ensure};
use crate::{claude::{paths::ClaudeDir, discover::discover_transcripts}, config::canonical_root};

#[derive(Clone, Debug)]
pub enum Trash { System, Dir(PathBuf) }
impl Trash {
    pub fn from_env() -> Self {
        std::env::var_os("ASM_TRASH_DIR").map(|p| Self::Dir(p.into())).unwrap_or(Self::System)
    }
}
#[derive(Clone, Debug)]
pub struct FavoritesDir { pub root: PathBuf, pub trash: Trash }
impl FavoritesDir { pub fn claude(&self) -> ClaudeDir { ClaudeDir::new(self.root.join("claude")) } }

fn validate_component(s: &str) -> Result<()> {
    ensure!(!s.is_empty() && !s.contains('/') && !s.contains("..") && !s.contains('\\'), "invalid session or project id"); Ok(())
}
fn transcript(root: &Path, project: &str, id: &str) -> Result<PathBuf> {
    validate_component(project)?; validate_component(id)?;
    Ok(root.join("claude/projects").join(project).join(format!("{id}.jsonl")))
}
pub fn original_exists(claude: &ClaudeDir, project: &str, id: &str) -> bool {
    claude.projects_dir().join(project).join(format!("{id}.jsonl")).is_file()
}
pub fn save(root: &Path, source: &Path, project: &str, id: &str) -> Result<()> {
    let target = transcript(root, project, id)?;
    ensure!(!discover_transcripts(&ClaudeDir::new(root.join("claude")))?.iter().any(|f| f.session_id == id), "already a favorite");
    ensure!(!target.with_extension("").try_exists()?, "orphan favorite directory exists; recover it before saving this session");
    let staging = root.join("claude/.staging").join(id);
    fs::create_dir_all(staging.parent().unwrap())?;
    fs::create_dir(&staging).context("create staging (an unfinished operation may need recovery)")?;
    let result = stage_and_publish(source, &staging, &target, id);
    if result.is_err() { let _ = fs::remove_dir_all(&staging); }
    result
}
fn stage_and_publish(source: &Path, staging: &Path, target: &Path, id: &str) -> Result<()> {
    let main = staging.join(format!("{id}.jsonl"));
    copy_file(source, &main)?;
    let source_dir = source.with_extension("");
    if source_dir.try_exists()? { copy_tree(&source_dir, &staging.join(id))?; }
    fs::create_dir_all(target.parent().unwrap())?;
    ensure!(!target.try_exists()?, "already a favorite");
    // An orphan directory is never silently overwritten.
    let staged_dir = staging.join(id);
    let target_dir = target.with_extension("");
    ensure!(!target_dir.try_exists()?, "orphan favorite directory exists");
    let has_directory = staged_dir.try_exists()?;
    if has_directory { fs::rename(&staged_dir, &target_dir)?; }
    if let Err(error) = fs::rename(main, target) {
        if has_directory { fs::rename(&target_dir, &staged_dir).context("restore staged session directory")?; }
        return Err(error.into());
    }
    fs::remove_dir(staging)?;
    Ok(())
}
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        ensure!(!entry.file_type()?.is_symlink(), "snapshot contains symlink: {}", entry.path().display());
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() { copy_tree(&entry.path(), &dest)?; } else { copy_file(&entry.path(), &dest)?; }
    }
    File::open(to)?.set_modified(fs::metadata(from)?.modified()?)?;
    Ok(())
}
fn copy_file(from: &Path, to: &Path) -> Result<()> {
    let modified = fs::metadata(from)?.modified()?;
    fs::copy(from, to)?;
    let mut file = fs::OpenOptions::new().read(true).write(true).open(to)?;
    if to.extension().is_some_and(|e| e == "jsonl") { truncate_partial_line(&mut file)?; }
    file.set_modified(modified)?;
    Ok(())
}
fn truncate_partial_line(file: &mut File) -> Result<()> {
    let mut end = file.metadata()?.len();
    let mut buffer = [0u8; 8192];
    while end > 0 {
        let start = end.saturating_sub(buffer.len() as u64);
        file.seek(SeekFrom::Start(start))?;
        let bytes = &mut buffer[..(end - start) as usize]; file.read_exact(bytes)?;
        if let Some(pos) = bytes.iter().rposition(|b| *b == b'\n') { file.set_len(start + pos as u64 + 1)?; return Ok(()); }
        end = start;
    }
    file.set_len(0)?; Ok(())
}
pub fn remove(root: &Path, project: &str, id: &str, trash: &Trash) -> Result<()> {
    let path = transcript(root, project, id)?;
    ensure!(path.try_exists()?, "not a favorite");
    let dir = path.with_extension("");
    let mut paths = vec![path]; if dir.try_exists()? { paths.push(dir); }
    match trash {
        Trash::System => {
            #[cfg(target_os = "macos")]
            {
                use trash::macos::{DeleteMethod, TrashContextExtMacos};
                let mut context = trash::TrashContext::default();
                context.set_delete_method(DeleteMethod::NsFileManager);
                for path in paths { context.delete(path)?; }
            }
            #[cfg(not(target_os = "macos"))]
            for path in paths { trash::delete(path)?; }
        }
        Trash::Dir(root) => {
            fs::create_dir_all(root)?;
            let destination = root.join(format!("{id}-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
            fs::create_dir(&destination)?;
            for path in paths { fs::rename(&path, destination.join(path.file_name().unwrap()))?; }
        }
    }
    Ok(())
}
#[derive(Default, Debug)]
pub struct Migration { pub moved: usize, pub skipped: usize }
pub fn validate_migration(old: &Path, new: &Path, claude: &ClaudeDir) -> Result<()> {
    let old = canonical_root(old)?; let new = canonical_root(new)?; let claude = canonical_root(claude.root())?;
    ensure!(!new.starts_with(&old) && !old.starts_with(&new), "favorites roots must not be equal or nested");
    ensure!(!new.starts_with(&claude), "favorites root must not be inside Claude directory"); Ok(())
}
pub fn migrate(old: &Path, new: &Path, claude: &ClaudeDir) -> Result<Migration> {
    migrate_with_commit(old, new, claude, || Ok(()))
}

/// Run configuration/index updates only after all files move; roll files back on any failure.
pub fn migrate_with_commit(old: &Path, new: &Path, claude: &ClaudeDir, commit: impl FnOnce() -> Result<()>) -> Result<Migration> {
    validate_migration(old, new, claude)?;
    let existing: std::collections::HashSet<_> = discover_transcripts(&ClaudeDir::new(new.join("claude")))?.into_iter().map(|f| f.session_id).collect();
    let files = discover_transcripts(&ClaudeDir::new(old.join("claude")))?;
    let mut report = Migration::default(); let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for f in files {
        if existing.contains(&f.session_id) { report.skipped += 1; continue; }
        let target = transcript(new, &f.project_key, &f.session_id)?;
        if let Err(error) = move_session(&f.path, &target) {
            for (from, to) in moved.iter().rev() {
                move_session(to, from).context(format!("migration failed: {error}; rollback failed"))?;
            }
            return Err(error);
        }
        moved.push((f.path, target)); report.moved += 1;
    }
    if let Err(error) = commit() {
        for (from, to) in moved.iter().rev() {
            move_session(to, from).context(format!("configuration failed: {error}; rollback failed"))?;
        }
        return Err(error);
    }
    Ok(report)
}
fn move_session(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to.parent().unwrap())?;
    let from_dir = from.with_extension(""); let to_dir = to.with_extension("");
    if from_dir.try_exists()? { move_path(&from_dir, &to_dir)?; }
    if let Err(error) = move_path(from, to) {
        if to_dir.try_exists()? { move_path(&to_dir, &from_dir).context("restore session directory")?; }
        return Err(error);
    }
    Ok(())
}
fn move_path(from: &Path, to: &Path) -> Result<()> {
    ensure!(!to.try_exists()?, "migration destination already exists: {}", to.display());
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(18) => {
            let staging = to.with_file_name(format!(".migration-{}-{}", to.file_name().unwrap().to_string_lossy(), std::process::id()));
            ensure!(!staging.try_exists()?, "unfinished migration staging exists");
            let copied = if from.is_dir() { copy_tree(from, &staging) } else { copy_file(from, &staging) };
            if let Err(error) = copied {
                if staging.is_dir() { fs::remove_dir_all(&staging)?; } else if staging.try_exists()? { fs::remove_file(&staging)?; }
                return Err(error);
            }
            fs::rename(&staging, to)?;
            if from.is_dir() { fs::remove_dir_all(from)?; } else { fs::remove_file(from)?; }
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}
