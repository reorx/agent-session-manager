mod common;
use common::*;
use agent_session_core::{favorites::{self, Trash}, claude::paths::ClaudeDir, index::{db::Db, indexer::{Indexer, ScanEvent}}};

fn scan(sb: &Sandbox, db: &mut Db) -> agent_session_core::index::indexer::ScanReport {
    Indexer::new(sb.claude.clone(), Some(ClaudeDir::new(sb.fav_root().join("claude")))).scan(db, &mut |_| {}).unwrap()
}
fn save(sb: &Sandbox) { favorites::save(&sb.fav_root(), &sb.transcript(ALPHA, S1), ALPHA, S1).unwrap(); }
fn remove(sb: &Sandbox) { favorites::remove(&sb.fav_root(), ALPHA, S1, &Trash::Dir(sb.tmp.path().join("trash"))).unwrap(); }

#[test]
fn favoriting_moves_the_row_to_the_copy_without_changing_it() {
    let sb = sandbox(); let mut db = Db::open_in_memory().unwrap(); scan(&sb, &mut db);
    let before = db.session(S1).unwrap().unwrap(); save(&sb);
    let mut rows = Vec::new();
    let report = Indexer::new(sb.claude.clone(), Some(ClaudeDir::new(sb.fav_root().join("claude"))))
        .scan_streaming(&mut db, &mut |e| if let ScanEvent::Session(s) = e { if s.id == S1 { rows.push(s); } }).unwrap();
    let after = db.session(S1).unwrap().unwrap();
    assert!(after.favorite); assert!(after.file_path.unwrap().starts_with(sb.fav_root()));
    assert_eq!(before.cost_usd, after.cost_usd); assert_eq!(after.updated_at, MTIME_S1);
    assert_eq!(db.messages(S1).unwrap().len(), 10); assert_eq!(after.subagent_count, 1);
    assert_eq!(report.files_missing, 0); assert_eq!(rows.len(), 1);
    assert!(!db.all_index_states().unwrap().contains_key(&sb.transcript(ALPHA, S1)));
}
#[test]
fn a_favorite_outlives_its_deleted_original() {
    let sb = sandbox(); let mut db = Db::open_in_memory().unwrap(); save(&sb); scan(&sb, &mut db);
    let before = db.session(S1).unwrap();
    std::fs::remove_file(sb.transcript(ALPHA, S1)).unwrap();
    std::fs::remove_dir_all(sb.transcript(ALPHA, S1).with_extension("")).unwrap();
    assert_eq!(scan(&sb, &mut db).files_missing, 0); assert_eq!(before, db.session(S1).unwrap());
}
#[test]
fn the_favorite_wins_and_unfavoriting_falls_back_to_the_grown_original() {
    let sb = sandbox(); let mut db = Db::open_in_memory().unwrap(); scan(&sb, &mut db); save(&sb); scan(&sb, &mut db);
    let before = db.session(S1).unwrap();
    append(&sb.transcript(ALPHA, S1), r#"{"type":"user","uuid":"grown","message":{"role":"user","content":"grown original"}}
"#);
    scan(&sb, &mut db); assert_eq!(before, db.session(S1).unwrap());
    remove(&sb); scan(&sb, &mut db);
    let after = db.session(S1).unwrap().unwrap(); assert!(!after.favorite);
    assert_eq!(after.last_prompt.as_deref(), Some("grown original")); assert_eq!(after.cost_usd, before.unwrap().cost_usd);
    let mut fresh = Db::open_in_memory().unwrap(); scan(&sb, &mut fresh);
    assert_eq!(db.session(S1).unwrap(), fresh.session(S1).unwrap()); assert_eq!(db.messages(S1).unwrap(), fresh.messages(S1).unwrap());
}
#[test]
fn unfavoriting_after_the_original_is_gone_leaves_a_history_only_row() {
    let sb = sandbox(); let mut db = Db::open_in_memory().unwrap(); save(&sb); scan(&sb, &mut db);
    let cost = db.session(S1).unwrap().unwrap().cost_usd;
    std::fs::remove_file(sb.transcript(ALPHA, S1)).unwrap();
    std::fs::remove_dir_all(sb.transcript(ALPHA, S1).with_extension("")).unwrap();
    remove(&sb); scan(&sb, &mut db); let s = db.session(S1).unwrap().unwrap();
    assert!(s.transcript_missing); assert!(!s.favorite); assert_eq!(s.cost_usd, cost); assert!(db.messages(S1).unwrap().is_empty());
}
#[test]
fn a_missing_favorites_root_is_not_an_error() { let sb = sandbox(); scan(&sb, &mut Db::open_in_memory().unwrap()); }
#[test]
fn favoriting_twice_is_an_error() { let sb = sandbox(); save(&sb); assert!(favorites::save(&sb.fav_root(), &sb.transcript(ALPHA, S1), ALPHA, S1).is_err()); }
#[test]
fn favoriting_a_growing_file_stops_at_the_last_newline_and_keeps_the_mtime() {
    let sb = sandbox(); let path = sb.transcript(ALPHA, S1); let len = std::fs::metadata(&path).unwrap().len();
    append(&path, "{\"unfinished\":"); set_mtime(&path, MTIME_S1); save(&sb);
    let copy = sb.fav_root().join("claude/projects").join(ALPHA).join(format!("{S1}.jsonl"));
    assert_eq!(std::fs::metadata(copy).unwrap().len(), len); assert!(std::fs::metadata(path).unwrap().len() > len);
}
#[test]
fn a_scan_with_other_roots_fails_with_roots_changed() {
    let sb = sandbox(); let mut db = Db::open_in_memory().unwrap(); scan(&sb, &mut db);
    assert!(Indexer::new(sb.claude.clone(), None).scan(&mut db, &mut |_| {}).unwrap_err().to_string().contains("RootsChanged"));
}
#[test]
fn an_unreadable_favorites_root_fails_the_scan_and_marks_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let sb = sandbox(); let mut db = Db::open_in_memory().unwrap(); save(&sb); scan(&sb, &mut db);
    let before = db.session(S1).unwrap(); let dir = sb.fav_root().join("claude/projects");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0)).unwrap();
    let result = Indexer::new(sb.claude.clone(), Some(ClaudeDir::new(sb.fav_root().join("claude")))).scan(&mut db, &mut |_| {});
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err()); assert_eq!(before, db.session(S1).unwrap());
}
#[test]
fn a_half_copied_favorite_is_ignored() {
    let sb = sandbox(); let mut db = Db::open_in_memory().unwrap(); scan(&sb, &mut db); let before = db.session(S1).unwrap();
    copy_tree(&sb.transcript(ALPHA, S1).with_extension(""), &sb.fav_root().join("claude/projects").join(ALPHA).join(S1));
    copy_tree(&sb.root(), &sb.fav_root().join("claude/.staging"));
    scan(&sb, &mut db); assert_eq!(before, db.session(S1).unwrap());
}
#[test]
fn migrating_the_root_reindexes_from_the_new_place() {
    let sb = sandbox(); let mut db = Db::open_in_memory().unwrap(); save(&sb); scan(&sb, &mut db);
    let new = sb.tmp.path().join("new");
    assert_eq!(favorites::migrate(&sb.fav_root(), &new, &sb.claude).unwrap().moved, 1);
    db.set_meta("favorites_root", &agent_session_core::config::canonical_root(&new.join("claude")).unwrap().to_string_lossy()).unwrap();
    Indexer::new(sb.claude.clone(), Some(ClaudeDir::new(new.join("claude")))).scan(&mut db, &mut |_| {}).unwrap();
    assert!(db.session(S1).unwrap().unwrap().file_path.unwrap().starts_with(new));
    assert_eq!(db.messages(S1).unwrap().len(), 10);
}
#[test]
fn migration_keeps_the_new_roots_copy_on_conflict() {
    let sb = sandbox(); save(&sb); let new = sb.tmp.path().join("new");
    favorites::save(&new, &sb.transcript(ALPHA, S1), BETA, S1).unwrap();
    let result = favorites::migrate(&sb.fav_root(), &new, &sb.claude).unwrap(); assert_eq!(result.skipped, 1); assert_eq!(result.moved, 0);
    assert!(sb.fav_root().join("claude/projects").join(ALPHA).join(format!("{S1}.jsonl")).exists());
}
#[test]
fn migration_rejects_nested_or_claude_roots() {
    let sb = sandbox(); save(&sb);
    for new in [sb.fav_root(), sb.fav_root().join("nested"), sb.tmp.path().to_path_buf(), sb.root().join("nested")] {
        assert!(favorites::migrate(&sb.fav_root(), &new, &sb.claude).is_err());
    }
}
#[test]
fn rebuild_keeps_favorites_whose_original_is_gone() {
    let sb = sandbox(); save(&sb); std::fs::remove_file(sb.transcript(ALPHA, S1)).unwrap();
    let mut db = Db::open_in_memory().unwrap();
    Indexer::new(sb.claude.clone(), Some(ClaudeDir::new(sb.fav_root().join("claude")))).rebuild(&mut db, &mut |_| {}).unwrap();
    assert!(db.session(S1).unwrap().unwrap().favorite); assert_eq!(db.messages(S1).unwrap().len(), 10);
}
#[test]
fn resolve_transcript_and_list_transcripts_see_favorites() {
    let sb = sandbox(); save(&sb); let fav = ClaudeDir::new(sb.fav_root().join("claude"));
    assert!(agent_session_core::transcript::resolve_transcript(&sb.claude, Some(&fav), S1).unwrap().starts_with(sb.fav_root()));
    let files = agent_session_core::search::regex::list_transcripts(&sb.claude, Some(&fav), None, None).unwrap();
    assert_eq!(files.iter().filter(|f| f.session_id == S1).count(), 1);
    assert!(files.iter().find(|f| f.session_id == S1).unwrap().favorite);
}
#[test]
fn failed_migration_rolls_back_already_moved_sessions() {
    let sb = sandbox(); save(&sb);
    favorites::save(&sb.fav_root(), &sb.transcript(ALPHA, S2), ALPHA, S2).unwrap();
    let new = sb.tmp.path().join("new");
    let blocker = new.join("claude/projects").join(ALPHA).join(S1);
    std::fs::create_dir_all(&blocker).unwrap(); std::fs::write(blocker.join("keep"), "existing").unwrap();
    assert!(favorites::migrate(&sb.fav_root(), &new, &sb.claude).is_err());
    for id in [S1, S2] {
        assert!(sb.fav_root().join("claude/projects").join(ALPHA).join(format!("{id}.jsonl")).exists());
    }
}
#[test]
fn configuration_failure_rolls_migration_back() {
    let sb = sandbox(); save(&sb); let new = sb.tmp.path().join("new");
    let result = favorites::migrate_with_commit(&sb.fav_root(), &new, &sb.claude, || anyhow::bail!("config is read-only"));
    assert!(result.unwrap_err().to_string().contains("config is read-only"));
    assert!(sb.fav_root().join("claude/projects").join(ALPHA).join(format!("{S1}.jsonl")).exists());
    assert!(!new.join("claude/projects").join(ALPHA).join(format!("{S1}.jsonl")).exists());
}
#[test]
fn a_fresh_index_equals_the_incremental_one_including_usage() {
    let sb = sandbox(); let mut db = Db::open_in_memory().unwrap(); scan(&sb, &mut db); save(&sb); scan(&sb, &mut db);
    let mut fresh = Db::open_in_memory().unwrap(); scan(&sb, &mut fresh);
    assert_eq!(db.session(S1).unwrap(), fresh.session(S1).unwrap());
    assert_eq!(db.messages(S1).unwrap(), fresh.messages(S1).unwrap());
    fn usage(db: &Db) -> Vec<(String, String, i64, i64, Option<f64>)> {
        db.conn().prepare("SELECT agent, msg_id, input, output, cost_usd FROM api_usage WHERE session_id = ?1 ORDER BY agent, msg_id").unwrap()
            .query_map([S1], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).unwrap().collect::<Result<_, _>>().unwrap()
    }
    assert_eq!(usage(&db), usage(&fresh));
}
#[test]
fn saving_does_not_adopt_an_orphan_subagent_directory() {
    let sb = sandbox();
    let orphan = sb.fav_root().join("claude/projects").join(ALPHA).join(S2).join("subagents");
    std::fs::create_dir_all(&orphan).unwrap(); std::fs::write(orphan.join("old.jsonl"), "{}\n").unwrap();
    assert!(favorites::save(&sb.fav_root(), &sb.transcript(ALPHA, S2), ALPHA, S2).is_err());
    assert!(!orphan.parent().unwrap().with_extension("jsonl").exists());
}
