//! Core logic for Agent Session Manager: transcript parsing, SQLite indexing and search.
//!
//! This crate must never depend on GPUI. The UI crate (`crates/app`) stays thin on top of it.
//! Behaviour follows `~/Code/scripts/claude_session_search.py`; see `kb/docs/data-layer-design.md`.

pub mod claude;
pub mod format;
pub mod index;
pub mod model;
pub mod pricing;
pub mod search;
pub mod transcript;

pub mod config;
pub mod favorites;
