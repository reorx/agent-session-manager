//! Configuration shared by the CLI and desktop app, next to the index.
use std::path::{Path, PathBuf};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
struct Config { favorites_dir: Option<PathBuf> }

pub fn default_favorites_dir(db: &Path) -> PathBuf { db.parent().unwrap_or(Path::new(".")).join("favorites") }
pub fn environment_override() -> bool { std::env::var_os("ASM_FAVORITES_DIR").is_some_and(|s| !s.is_empty()) }
pub fn favorites_dir(db: &Path) -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("ASM_FAVORITES_DIR").filter(|s| !s.is_empty()) { return Ok(dir.into()); }
    let path = db.with_file_name("config.json");
    let config: Config = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => return Err(e.into()),
    };
    Ok(config.favorites_dir.unwrap_or_else(|| default_favorites_dir(db)))
}
pub fn set_favorites_dir(db: &Path, dir: Option<&Path>) -> Result<()> {
    anyhow::ensure!(!environment_override(), "Favorites directory is controlled by ASM_FAVORITES_DIR");
    let path = db.with_file_name("config.json");
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(&Config { favorites_dir: dir.map(Path::to_path_buf) })?)?;
    std::fs::rename(temp, path)?;
    Ok(())
}
/// Canonical identity even when a configured directory has not been created yet.
pub fn canonical_root(path: &Path) -> Result<PathBuf> {
    match path.canonicalize() {
        Ok(p) => Ok(p),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let absolute = std::path::absolute(path)?;
            let parent = canonical_root(absolute.parent().ok_or_else(|| anyhow::anyhow!("invalid root"))?)?;
            Ok(parent.join(absolute.file_name().ok_or_else(|| anyhow::anyhow!("invalid root"))?))
        }
        Err(e) => Err(e.into()),
    }
}
