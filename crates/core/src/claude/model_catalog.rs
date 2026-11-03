//! Read-only context windows from Claude Code's local model-catalog cache.
//! Expiry is deliberately ignored: an old declaration still supplements the built-in table.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use super::paths::ClaudeDir;
use crate::pricing::normalize;

#[derive(Clone, Debug, Default)]
pub struct ModelCatalog {
    windows: BTreeMap<String, u64>,
    files: Vec<CatalogFile>,
}

#[derive(Clone, Debug)]
pub struct CatalogFile {
    pub path: PathBuf,
    pub kind: CatalogKind,
    pub fetched_at: Option<i64>,
    pub models: usize,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CatalogKind {
    Served,
    Published,
}

impl ModelCatalog {
    /// The sole error boundary: unavailable directories mean an empty catalog; bad files
    /// are retained for diagnostics while other files continue to contribute windows.
    pub fn load(claude: &ClaudeDir) -> Self {
        let Ok(paths) = catalog_paths(&claude.model_catalog_dir()) else { return Self::default() };
        let mut parsed = Vec::new();
        for (path, kind) in paths {
            let mut file = CatalogFile { path, kind, fetched_at: None, models: 0, error: None };
            let windows = match read_catalog(&file.path, kind) {
                Ok((fetched_at, windows)) => {
                    file.fetched_at = fetched_at;
                    file.models = windows.len();
                    windows
                }
                Err(error) => {
                    file.error = Some(format!("{error:#}"));
                    BTreeMap::new()
                }
            };
            parsed.push((file, windows));
        }
        parsed.sort_by(|(a, _), (b, _)| a.kind.cmp(&b.kind).then_with(|| b.fetched_at.cmp(&a.fetched_at)).then_with(|| a.path.cmp(&b.path)));
        let mut catalog = Self::default();
        for (file, windows) in parsed {
            for (id, window) in windows {
                catalog.windows.entry(id).or_insert(window);
            }
            catalog.files.push(file);
        }
        catalog
    }

    pub fn window(&self, model: &str) -> Option<u64> {
        self.windows.get(normalize(model)).copied()
    }

    pub fn windows(&self) -> &BTreeMap<String, u64> { &self.windows }
    pub fn files(&self) -> &[CatalogFile] { &self.files }

    /// Also useful to callers constructing a catalog without disk I/O. Applies the same
    /// validation and canonical-id preference as a single cache file.
    pub fn from_windows<'a>(windows: impl IntoIterator<Item = (&'a str, u64)>) -> Self {
        Self { windows: normalize_windows(windows.into_iter().map(|(id, w)| (id.to_owned(), w))), files: Vec::new() }
    }
}

fn catalog_paths(dir: &Path) -> Result<Vec<(PathBuf, CatalogKind)>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if let Some(kind) = name.to_str().and_then(catalog_kind) {
            paths.push((entry.path(), kind));
        }
    }
    Ok(paths)
}

fn catalog_kind(name: &str) -> Option<CatalogKind> {
    if !name.ends_with(".json") || name == "published-floor.json" { return None; }
    if name.starts_with("published-") { Some(CatalogKind::Published) }
    else if name.ends_with("-cc.json") { Some(CatalogKind::Served) }
    else { None }
}

// All fields are optional so additions or unrelated metadata do not affect window lookup.
#[derive(Deserialize)]
struct Cache {
    #[serde(rename = "fetchedAt")]
    fetched_at: Option<i64>,
    catalog: Option<Served>,
    document: Option<Document>,
}

#[derive(Deserialize)]
struct Served { config: Option<Config> }
#[derive(Deserialize)]
struct Config { models: Option<Vec<Model>> }
#[derive(Deserialize)]
struct Document { surfaces: Option<Surfaces> }
#[derive(Deserialize)]
struct Surfaces { cc: Option<Surface> }
#[derive(Deserialize)]
struct Surface { model_selector_config: Option<Vec<Config>> }
#[derive(Deserialize)]
struct Model { id: Option<String>, runtime: Option<Runtime>, context_window: Option<f64> }
#[derive(Deserialize)]
struct Runtime { max_input_tokens: Option<f64> }

fn read_catalog(path: &Path, kind: CatalogKind) -> Result<(Option<i64>, BTreeMap<String, u64>)> {
    let bytes = std::fs::read(path)?;
    let (fetched_at, models) = parse_catalog(&bytes, kind)?;
    let windows = normalize_windows(models.into_iter().filter_map(|model| {
        let window = model.runtime.and_then(|r| r.max_input_tokens).or(model.context_window)?;
        if !window.is_finite() || window.fract() != 0.0 { return None; }
        Some((model.id?, window as u64))
    }));
    Ok((fetched_at, windows))
}

fn parse_catalog(bytes: &[u8], kind: CatalogKind) -> Result<(Option<i64>, Vec<Model>)> {
    let cache: Cache = serde_json::from_slice(bytes)?;
    let models = match kind {
        CatalogKind::Served => cache.catalog.and_then(|c| c.config).and_then(|c| c.models).context("missing catalog.config.models")?,
        CatalogKind::Published => cache.document.and_then(|d| d.surfaces).and_then(|s| s.cc)
            .and_then(|s| s.model_selector_config).context("missing document.surfaces.cc.model_selector_config")?
            .into_iter().flat_map(|c| c.models.unwrap_or_default()).collect(),
    };
    Ok((cache.fetched_at, models))
}

fn normalize_windows(windows: impl IntoIterator<Item = (String, u64)>) -> BTreeMap<String, u64> {
    let mut normalized = BTreeMap::new();
    for (id, window) in windows {
        if !(100_000..=10_000_000).contains(&window) { continue; }
        let id = id.trim().to_lowercase();
        let key = normalize(&id);
        if key.is_empty() { continue; }
        if key == id || !normalized.contains_key(key) {
            normalized.insert(key.to_owned(), window);
        }
    }
    normalized
}
