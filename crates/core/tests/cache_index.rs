//! Persistence lifecycle through the real scanner, separate from replay's record-boundary tests.
mod common;
use agent_session_core::{claude::{conversation::read_conversation, parse::TranscriptParser}, index::{Db, Indexer}, model::*};
use common::*;
use std::{fs, path::Path};

const FIXTURE: &[u8] = include_bytes!("fixtures/cache_store.jsonl");

fn scan(sb: &Sandbox, db: &mut Db) { Indexer::new(sb.claude.clone(), None).scan(db, &mut |_| {}).unwrap(); }

#[test]
fn cache_store_scanner_incremental_every_byte_including_partial_lines_matches_full() {
    let sb = sandbox();
    let path = sb.transcript(ALPHA, S1);
    let mut db = Db::open(&sb.db_path()).unwrap();
    fs::write(&path, FIXTURE).unwrap(); scan(&sb, &mut db);
    let events = db.cache_events(S1).unwrap();
    let calls = db.cache_calls(S1).unwrap();
    let breaks = db.cache_breaks(S1).unwrap();
    assert_eq!(calls.len(), 3); assert_eq!(breaks.len(), 2);
    fs::write(&path, []).unwrap(); scan(&sb, &mut db);
    for (i, byte) in FIXTURE.iter().enumerate() {
        append(&path, &(*byte as char).to_string());
        // Every half-line end and full record boundary; the parser is recreated each scan.
        if *byte == b'\n' || i % 79 == 0 { scan(&sb, &mut db); }
    }
    scan(&sb, &mut db);
    assert_eq!(db.cache_events(S1).unwrap(), events);
    assert_eq!(db.cache_calls(S1).unwrap(), calls);
    assert_eq!(db.cache_breaks(S1).unwrap(), breaks);
}

#[test]
fn cache_store_replacement_truncation_missing_reappearance_and_true_delete() {
    let sb = sandbox(); let path = sb.transcript(ALPHA, S1);
    let mut db = Db::open(&sb.db_path()).unwrap();
    fs::write(&path, FIXTURE).unwrap(); scan(&sb, &mut db);
    let original = db.cache_breaks(S1).unwrap(); assert_eq!(original.len(), 2);
    drop(db);
    let mut db = Db::open(&sb.db_path()).unwrap();
    fs::remove_file(&path).unwrap(); scan(&sb, &mut db);
    assert!(db.messages(S1).unwrap().is_empty());
    assert_eq!(db.cache_breaks(S1).unwrap(), original);
    db.refresh_cache_breaks(S1).unwrap(); // No transcript exists; replay still works.
    assert_eq!(db.cache_breaks(S1).unwrap(), original);
    fs::write(&path, FIXTURE).unwrap(); scan(&sb, &mut db);
    assert_eq!(db.cache_breaks(S1).unwrap(), original);
    let replacement = path.with_extension("replacement");
    fs::write(&replacement, b"{\"type\":\"user\",\"message\":{\"content\":\"replacement\"}}\n").unwrap();
    fs::rename(&replacement, &path).unwrap(); scan(&sb, &mut db);
    assert!(db.cache_calls(S1).unwrap().is_empty()); assert!(db.cache_breaks(S1).unwrap().is_empty());
    fs::write(&path, FIXTURE).unwrap(); scan(&sb, &mut db);
    fs::write(&path, &FIXTURE[..FIXTURE.iter().position(|b| *b == b'\n').unwrap()+1]).unwrap(); scan(&sb, &mut db);
    assert!(db.cache_breaks(S1).unwrap().is_empty());
    fs::write(&path, FIXTURE).unwrap(); scan(&sb, &mut db);
    db.delete_session(S1).unwrap();
    assert!(db.cache_events(S1).unwrap().is_empty()); assert!(db.cache_calls(S1).unwrap().is_empty()); assert!(db.cache_breaks(S1).unwrap().is_empty());
}

#[test]
fn cache_store_subagent_deletion_and_schema_reset_do_not_mix_with_main_facts() {
    let sb = sandbox(); let path = sb.transcript(ALPHA, S1);
    fs::write(&path, FIXTURE).unwrap();
    let sub = path.parent().unwrap().join(S1).join("subagents/agent-cache.jsonl");
    fs::create_dir_all(sub.parent().unwrap()).unwrap(); fs::write(&sub, FIXTURE).unwrap();
    let mut db = Db::open(&sb.db_path()).unwrap(); scan(&sb, &mut db);
    let original = db.cache_breaks(S1).unwrap();
    let before = db.session(S1).unwrap().unwrap().tokens_cache_write;
    fs::remove_file(sub).unwrap(); scan(&sb, &mut db);
    assert!(db.session(S1).unwrap().unwrap().tokens_cache_write < before);
    assert_eq!(db.cache_breaks(S1).unwrap(), original);
    db.clear_all().unwrap(); assert!(db.cache_events(S1).unwrap().is_empty()); assert!(db.cache_calls(S1).unwrap().is_empty());
    scan(&sb, &mut db); db.set_user_version_for_test(7); drop(db);
    let db = Db::open(&sb.db_path()).unwrap(); assert!(db.cache_breaks(S1).unwrap().is_empty());
}

#[test]
fn cache_store_generated_fixtures_agree_with_reader_and_never_store_text() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cache_store.jsonl");
    let conv = read_conversation(&fixture, None).unwrap();
    let feed = TranscriptParser::new("sample").feed(FIXTURE, 0);
    let data = serde_json::to_string(&feed.cache_events).unwrap();
    assert!(!data.contains("Check cache facts"));
    let expected: Vec<_> = conv.items.into_iter().flat_map(|i| match i { ConvItem::Turn(t) => t.cache_breaks, _ => vec![] }).collect();
    let sb = sandbox(); fs::write(sb.transcript(ALPHA, S1), FIXTURE).unwrap();
    let mut db = Db::open(&sb.db_path()).unwrap(); scan(&sb, &mut db);
    assert_eq!(db.cache_breaks(S1).unwrap().into_iter().map(|b| b.rebuilt).collect::<Vec<_>>(), expected);
}

#[path = "support/cache_audit.rs"]
mod cache_audit;
#[test]
#[ignore = "requires SHA256-verified frozen manifest and scratch paths"]
fn real_indexed_cache_breaks() { cache_audit::run_index(); }

#[test]
#[ignore = "requires an explicitly isolated ASM_CACHE_BENCH_DB"]
fn cache_store_replay_timings() {
    use std::time::Instant;
    let path = std::env::var("ASM_CACHE_BENCH_DB").expect("set isolated ASM_CACHE_BENCH_DB");
    let db = Db::open(Path::new(&path)).unwrap();
    let mut sessions = db.list_sessions(&Default::default()).unwrap();
    sessions.sort_by_key(|s| std::cmp::Reverse(s.file_size));
    let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let most_events: String = conn.query_row("SELECT session_id FROM cache_events GROUP BY session_id ORDER BY count(*) DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    for (label, session) in [("largest-file", &sessions.first().unwrap().id), ("most-events", &most_events)] {
        let expected = db.cache_breaks(session).unwrap();
        let events = db.cache_events(session).unwrap().len();
        for run in 1..=3 {
            let start = Instant::now();
            db.transaction(|db| db.refresh_cache_breaks(session)).unwrap();
            let ms = start.elapsed().as_secs_f64() * 1000.;
            assert_eq!(db.cache_breaks(session).unwrap(), expected);
            eprintln!("{label} replay run={run} events={events} calls={} elapsed_ms={ms:.3}", db.cache_calls(session).unwrap().len());
        }
    }
    let start = Instant::now();
    db.transaction(|db| {
        for session in &sessions { db.refresh_cache_breaks(&session.id)?; }
        Ok(())
    }).unwrap();
    eprintln!("all-session replay sessions={} elapsed_ms={:.3}", sessions.len(), start.elapsed().as_secs_f64() * 1000.);
}
