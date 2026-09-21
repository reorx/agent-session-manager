//! View state that survives restarts: filters, sort, preview mode, selection. Stored as JSON
//! next to the index (`state.json`); anything unreadable falls back to the defaults.

use std::path::{Path, PathBuf};

use agent_session_core::claude::paths::default_db_path;
use anyhow::Result;
use gpui_kit::component::table::ColumnSort;
use serde::{Deserialize, Serialize};

use crate::model::{DisplayMode, RangePreset};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub fn from_column(sort: ColumnSort) -> Option<SortDir> {
        match sort {
            ColumnSort::Ascending => Some(SortDir::Asc),
            ColumnSort::Descending => Some(SortDir::Desc),
            ColumnSort::Default => None,
        }
    }

    pub fn to_column(self) -> ColumnSort {
        match self {
            SortDir::Asc => ColumnSort::Ascending,
            SortDir::Desc => ColumnSort::Descending,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    pub mode: DisplayMode,
    /// Column key + direction; `None` is the default order (Updated, newest first).
    pub sort: Option<(String, SortDir)>,
    pub project_key: Option<String>,
    pub range: RangePreset,
    pub show_empty: bool,
    pub detail_visible: bool,
    pub selected_id: Option<String>,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            mode: DisplayMode::default(),
            sort: None,
            project_key: None,
            range: RangePreset::default(),
            show_empty: false,
            detail_visible: true,
            selected_id: None,
        }
    }
}

/// `$ASM_STATE`, else `state.json` next to the index.
pub fn default_state_path() -> PathBuf {
    match std::env::var_os("ASM_STATE").filter(|v| !v.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => default_db_path().with_file_name("state.json"),
    }
}

impl ViewState {
    pub fn load(path: &Path) -> ViewState {
        std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_state_comes_back_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/state.json");
        let state = ViewState {
            mode: DisplayMode::LastMessage,
            sort: Some(("prompts".into(), SortDir::Desc)),
            project_key: Some("-Users-me-Code-alpha".into()),
            range: RangePreset::Days7,
            show_empty: true,
            detail_visible: false,
            selected_id: Some("11111111-aaaa-4bbb-8ccc-000000000001".into()),
        };
        state.save(&path).unwrap();
        assert_eq!(ViewState::load(&path), state);
    }

    #[test]
    fn a_missing_or_corrupt_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ViewState::load(&dir.path().join("nope.json")), ViewState::default());
        let path = dir.path().join("state.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(ViewState::load(&path), ViewState::default());
    }

    #[test]
    fn fields_added_later_default_when_an_old_file_lacks_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, r#"{"mode":"Title","some_future_field":1}"#).unwrap();
        let state = ViewState::load(&path);
        assert_eq!(state.mode, DisplayMode::Title);
        assert!(state.detail_visible);
        assert_eq!(state.range, RangePreset::Any);
    }

    #[test]
    fn sort_directions_map_to_the_table_and_back() {
        assert_eq!(SortDir::from_column(ColumnSort::Default), None);
        assert_eq!(SortDir::from_column(SortDir::Asc.to_column()), Some(SortDir::Asc));
        assert_eq!(SortDir::from_column(SortDir::Desc.to_column()), Some(SortDir::Desc));
    }
}
