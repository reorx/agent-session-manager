//! Behavior: local Claude Code catalogs supply windows, with the built-in table as fallback.
mod common;

use agent_session_core::claude::{model_catalog::ModelCatalog, paths::ClaudeDir};
use agent_session_core::pricing::{builtin_window, context_window};
use serde_json::{Value, json};
use std::path::Path;

fn write(root: &Path, name: &str, value: Value) {
    let dir = root.join("cache/model-catalog");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
}

fn model(id: &str, window: f64) -> Value {
    json!({"id": id, "runtime": {"max_input_tokens": window}})
}

fn served(fetched: Option<i64>, models: Vec<Value>) -> Value {
    json!({"fetchedAt": fetched, "catalog": {"config": {"models": models}}})
}

#[test]
fn served_and_published_cc_models_are_merged_and_unknown_models_fall_back() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = ClaudeDir::new(tmp.path());
    common::copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/model-catalog"), &claude.model_catalog_dir());
    let catalog = ModelCatalog::load(&claude);
    assert_eq!(catalog.files().len(), 2);
    assert!(catalog.files().iter().all(|f| f.error.is_none()));
    for (id, window) in [("claude-opus-5-5", 1_000_000), ("claude-haiku-4-5-20251001", 200_000),
        ("claude-haiku-4-5", 200_000), ("claude-opus-5[1m]", 1_000_000),
        ("claude-opus-4-1-20250805", 200_000), ("claude-opus-6", 1_000_000)] {
        assert_eq!(context_window(&catalog, id), Some(window), "{id}");
    }
    assert_eq!(catalog.window("claude-opus-4-5"), None, "ignore chat");
    assert_eq!(context_window(&catalog, "claude-opus-4-5-20251101"), Some(200_000));
    assert_eq!(context_window(&catalog, "claude-sonnet-4-5"), Some(200_000));
    for id in ["claude-opus-9", "<synthetic>"] {
        assert_eq!(context_window(&catalog, id), None);
    }
}

#[test]
fn missing_or_unreadable_directory_keeps_all_builtin_windows() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = ClaudeDir::new(tmp.path());
    for blocked in [false, true] {
        if blocked {
            std::fs::create_dir_all(tmp.path().join("cache")).unwrap();
            std::fs::write(claude.model_catalog_dir(), "not a directory").unwrap();
        }
        let catalog = ModelCatalog::load(&claude);
        assert!(catalog.windows().is_empty());
        for id in ["claude-fable-5-1", "claude-mythos-5-1", "claude-fable-5", "claude-mythos-5",
            "claude-opus-5-5", "claude-opus-5", "claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6",
            "claude-opus-4-5", "claude-opus-4-1", "claude-opus-4", "claude-sonnet-5-5", "claude-sonnet-5",
            "claude-sonnet-4-6", "claude-sonnet-4-5", "claude-sonnet-4", "claude-haiku-5-5", "claude-haiku-4-5", "claude-3-5-haiku"] {
            assert_eq!(context_window(&catalog, id), builtin_window(id));
        }
    }
}

#[test]
fn bad_files_are_reported_without_discarding_good_files_and_irrelevant_files_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "good-cc.json", served(None, vec![model("claude-opus-5", 500_000.0)]));
    let dir = tmp.path().join("cache/model-catalog");
    std::fs::write(dir.join("broken-cc.json"), "{\"catalog\":").unwrap();
    for name in ["published-missing.json", "missing-cc.json"] { write(tmp.path(), name, json!({})); }
    for name in ["published-floor.json", "masked-ids.json", "masked-ids.unparseable.json", "x-cc.json.tmp.1", "other-ccd.json"] {
        std::fs::write(dir.join(name), "broken").unwrap();
    }
    let catalog = ModelCatalog::load(&ClaudeDir::new(tmp.path()));
    assert_eq!(catalog.files().len(), 4);
    assert_eq!(catalog.files().iter().filter(|f| f.error.is_some()).count(), 3);
    assert_eq!(context_window(&catalog, "claude-opus-5"), Some(500_000));
}

#[test]
fn only_valid_integer_windows_are_used_and_legacy_context_window_is_supported() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "test-cc.json", served(None, vec![
        model("claude-opus-5", 99_999.0), model("claude-sonnet-5", 10_000_001.0), model("claude-haiku-5-5", 150_000.5),
        json!({"id": "claude-opus-4-5"}), json!({"runtime": {"max_input_tokens": 500000}}),
        json!({"id": "claude-opus-6", "context_window": 500000}),
        json!({"id": "claude-opus-7", "runtime": {}, "context_window": 600000}),
        model("claude-lower", 100_000.0), model("claude-upper", 10_000_000.0),
    ]));
    let catalog = ModelCatalog::load(&ClaudeDir::new(tmp.path()));
    assert_eq!(catalog.windows().len(), 4);
    assert_eq!(catalog.window("claude-opus-6"), Some(500_000));
    assert_eq!(catalog.window("claude-opus-7"), Some(600_000));
    for id in ["claude-opus-5", "claude-sonnet-5", "claude-haiku-5-5", "claude-opus-4-5"] {
        assert_eq!(context_window(&catalog, id), builtin_window(id));
    }
}

#[test]
fn newest_served_wins_then_older_served_then_published_even_when_expired() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "old-cc.json", served(Some(10), vec![model("claude-opus-5", 200_000.0), model("older-only", 300_000.0)]));
    write(tmp.path(), "new-cc.json", served(Some(20), vec![model("claude-opus-5", 500_000.0)]));
    write(tmp.path(), "undated-cc.json", served(None, vec![model("claude-opus-5", 800_000.0), model("undated-only", 900_000.0)]));
    write(tmp.path(), "published-test.json", json!({"fetchedAt": 30, "staleAt": 0,
        "document": {"expires_at": "2000-01-01", "surfaces": {"cc": {"model_selector_config": [{"models": [
            model("claude-opus-5", 1_000_000.0), model("published-only", 400_000.0)]}]}}}}));
    write(tmp.path(), "published-old.json", json!({"fetchedAt": 5,
        "document": {"surfaces": {"cc": {"model_selector_config": [{"models": [model("published-only", 200_000.0)]}]}}}}));
    let catalog = ModelCatalog::load(&ClaudeDir::new(tmp.path()));
    assert_eq!(catalog.window("claude-opus-5"), Some(500_000));
    assert_eq!(catalog.window("older-only"), Some(300_000));
    assert_eq!(catalog.window("undated-only"), Some(900_000));
    assert_eq!(catalog.window("published-only"), Some(400_000));
}

#[test]
fn normalization_prefers_the_canonical_id_within_one_file() {
    let values = [(" CLAUDE-OPUS-5-20261103[1m] ", 200_000), ("claude-opus-5", 500_000), ("claude-opus-5-20261101", 700_000),
        ("claude-sonnet-5-20261103", 300_000), ("claude-sonnet-5-20261101", 400_000), ("", 500_000), ("bad", 1)];
    let catalog = ModelCatalog::from_windows(values);
    assert_eq!(catalog.windows().len(), 2);
    assert_eq!(catalog.window("claude-opus-5[1m]"), Some(500_000));
    assert_eq!(catalog.window("claude-sonnet-5"), Some(300_000));
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "test-cc.json", served(None, values.into_iter().map(|(id, w)| model(id, w as f64)).collect()));
    assert_eq!(ModelCatalog::load(&ClaudeDir::new(tmp.path())).windows(), catalog.windows());
}
