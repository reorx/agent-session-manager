//! Behavior: the SQLite index reflects the transcripts + history on disk, and rescans only touch
//! what changed (append / rewrite / delete / new file).

mod common;

use agent_session_core::index::{Db, Indexer, ListFilter, Order};
use agent_session_core::model::{Role, Source};
use common::*;

fn ms(iso: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(iso).unwrap().timestamp_millis()
}

fn scan(sb: &Sandbox, db: &mut Db) -> agent_session_core::index::ScanReport {
    Indexer::new(sb.claude.clone()).scan(db, &mut |_| {}).unwrap()
}

#[test]
fn first_scan_indexes_transcripts_and_fills_gaps_from_history() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    let report = scan(&sb, &mut db);
    assert_eq!(report.files_seen, 4, "top-level *.jsonl only; subagents and memory are not sessions");
    assert_eq!(report.files_parsed, 4);
    assert_eq!(report.files_unchanged, 0);
    assert_eq!(report.history_sessions, 4, "S4, S6 and two anonymous beta sessions");

    let all = db.list_sessions(&ListFilter::default()).unwrap();
    assert_eq!(all.len(), 8);
    let s1 = db.session(S1).unwrap().unwrap();
    assert_eq!(s1.source, Source::Transcript);
    assert_eq!(s1.user_msg_count, 3);
    assert_eq!(s1.subagent_count, 1);
    assert_eq!(s1.title.as_deref(), Some("Parser refactor"));
    assert_eq!(s1.updated_at, MTIME_S1);
    assert_eq!(s1.project_path.as_deref(), Some("/Users/me/Code/alpha"));
    assert_eq!(s1.tool_counts.get("Bash"), Some(&2));
    assert_eq!(s1.files_touched.len(), 1);
    assert_eq!(db.messages(S1).unwrap().len(), 10);

    let s4 = db.session(S4).unwrap().unwrap();
    assert_eq!(s4.source, Source::History);
    assert_eq!(s4.user_msg_count, 4);
    assert_eq!(s4.first_prompt, "Fix the flaky test\n\nline1\nline2\nline3");
    assert!(db.session(S6).unwrap().is_some(), "project known only from history");

    let mut projects = db.projects().unwrap();
    projects.sort_by(|a, b| a.key.cmp(&b.key));
    let keys: Vec<(&str, Option<&str>, usize)> =
        projects.iter().map(|p| (p.key.as_str(), p.path.as_deref(), p.session_count)).collect();
    assert_eq!(
        keys,
        vec![
            (ALPHA, Some("/Users/me/Code/alpha"), 4),
            (BETA, Some("/Users/me/Code/beta"), 3),
            (GAMMA, Some("/Users/me/Code/gamma"), 1),
        ]
    );
}

#[test]
fn list_filters_by_project_and_time_and_orders_like_the_reference() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);

    let alpha = db
        .list_sessions(&ListFilter { project_keys: Some(vec![ALPHA.into()]), ..Default::default() })
        .unwrap();
    let ids: Vec<&str> = alpha.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec![S2, S1, S4, S5], "started desc, sessions without a start last");

    let recent = db
        .list_sessions(&ListFilter {
            project_keys: Some(vec![ALPHA.into()]),
            since_ms: Some(ms("2026-09-02T00:00:00Z")),
            ..Default::default()
        })
        .unwrap();
    let ids: Vec<&str> = recent.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec![S2, S5], "filter applies to updated_at");

    let by_updated = db
        .list_sessions(&ListFilter { order: Order::UpdatedDesc, ..Default::default() })
        .unwrap();
    let ids: Vec<&str> = by_updated.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(&ids[..4], &[S6, S3, S2, S5]);
}

#[test]
fn prefix_lookup_finds_transcript_and_history_sessions() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    assert_eq!(db.find_by_prefix("4444").unwrap()[0].id, S4);
    assert_eq!(db.find_by_prefix("11111111-aaaa").unwrap()[0].id, S1);
    assert!(db.find_by_prefix("zzz").unwrap().is_empty());
    let anon = db.find_by_prefix("").unwrap();
    assert_eq!(anon.len(), 8, "empty prefix matches everything, anonymous ones included");
}

#[test]
fn unchanged_files_are_skipped_on_the_next_scan() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let report = scan(&sb, &mut db);
    assert_eq!(report.files_unchanged, 4);
    assert_eq!(report.files_parsed, 0);
    assert_eq!(db.list_sessions(&ListFilter::default()).unwrap().len(), 8, "history import is idempotent");
    assert_eq!(db.messages(S1).unwrap().len(), 10);
}

#[test]
fn appended_lines_are_parsed_incrementally() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let path = sb.transcript(ALPHA, S2);
    let before = std::fs::metadata(&path).unwrap().len();
    append(&path, concat!(
        r#"{"parentUuid":"x","isSidechain":false,"type":"user","message":{"role":"user","content":"And now in French"},"uuid":"u-new","timestamp":"2026-09-06T00:00:00.000Z","sessionId":"22222222-aaaa-4bbb-8ccc-000000000002","cwd":"/Users/me/Code/alpha","gitBranch":"main","entrypoint":"sdk-cli","version":"2.1.261"}"#, "\n",
        r#"{"parentUuid":"u-new","isSidechain":false,"message":{"model":"claude-haiku-4-5-20251001","id":"m","type":"message","role":"assistant","content":[{"type":"text","text":"Bonjour mon ami."}],"usage":{"input_tokens":1,"output_tokens":2}},"type":"assistant","uuid":"a-new","timestamp":"2026-09-06T00:00:01.000Z","sessionId":"22222222-aaaa-4bbb-8ccc-000000000002","cwd":"/Users/me/Code/alpha","gitBranch":"main","version":"2.1.261"}"#, "\n",
    ));
    set_mtime(&path, MTIME_S2 + 60_000);

    let report = scan(&sb, &mut db);
    assert_eq!(report.files_appended, 1);
    assert_eq!(report.files_unchanged, 3);
    assert_eq!(report.bytes_parsed, std::fs::metadata(&path).unwrap().len() - before, "only the tail is parsed");

    let s2 = db.session(S2).unwrap().unwrap();
    assert_eq!(s2.user_msg_count, 2);
    assert_eq!(s2.first_prompt, "Say hello in exactly three words.");
    assert_eq!(s2.last_prompt.as_deref(), Some("And now in French"));
    assert_eq!(s2.last_message.as_deref(), Some("Bonjour mon ami."));
    assert_eq!(s2.updated_at, MTIME_S2 + 60_000);
    let msgs = db.messages(S2).unwrap();
    assert_eq!(msgs.len(), 4);
    assert_eq!(msgs[2].byte_offset, before);
    assert_eq!(msgs[3].role, Role::Assistant);
}

#[test]
fn a_partial_last_line_waits_for_its_newline() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let path = sb.transcript(ALPHA, S2);
    let size = std::fs::metadata(&path).unwrap().len();
    let line = r#"{"parentUuid":"x","isSidechain":false,"type":"user","message":{"role":"user","content":"Half written"},"uuid":"u-half","timestamp":"2026-09-06T00:00:00.000Z","sessionId":"22222222-aaaa-4bbb-8ccc-000000000002"}"#;
    append(&path, &line[..line.len() / 2]);
    set_mtime(&path, MTIME_S2 + 1_000);
    scan(&sb, &mut db);
    assert_eq!(db.session(S2).unwrap().unwrap().user_msg_count, 1);
    assert_eq!(db.index_state(&path).unwrap().unwrap().indexed_offset, size, "offset stays at the last newline");

    append(&path, &line[line.len() / 2..]);
    append(&path, "\n");
    set_mtime(&path, MTIME_S2 + 2_000);
    let report = scan(&sb, &mut db);
    assert_eq!(report.files_appended, 1);
    assert_eq!(db.session(S2).unwrap().unwrap().user_msg_count, 2);
    assert_eq!(db.session(S2).unwrap().unwrap().last_prompt.as_deref(), Some("Half written"));
}

#[test]
fn a_rewritten_file_is_reindexed_from_scratch() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let path = sb.transcript(ALPHA, S1);
    let s2_bytes = std::fs::read(sb.transcript(ALPHA, S2)).unwrap();
    let rewritten = String::from_utf8(s2_bytes).unwrap().replace(S2, S1);
    std::fs::write(&path, rewritten).unwrap();
    set_mtime(&path, MTIME_S1 + 1_000);

    let report = scan(&sb, &mut db);
    assert_eq!(report.files_rebuilt, 1);
    let s1 = db.session(S1).unwrap().unwrap();
    assert_eq!(s1.user_msg_count, 1);
    assert_eq!(s1.first_prompt, "Say hello in exactly three words.");
    assert_eq!(s1.title, None, "stale fields from the old content are gone");
    assert_eq!(s1.tool_use_count, 0);
    assert_eq!(db.messages(S1).unwrap().len(), 2);
}

#[test]
fn a_deleted_transcript_keeps_its_row_marked_missing_and_drops_its_messages() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    std::fs::remove_file(sb.transcript(ALPHA, S1)).unwrap();
    let report = scan(&sb, &mut db);
    assert_eq!(report.files_missing, 1);
    let s1 = db.session(S1).unwrap().unwrap();
    assert!(s1.transcript_missing);
    assert_eq!(s1.user_msg_count, 3, "metadata gathered while the file existed is kept");
    assert!(db.messages(S1).unwrap().is_empty());
    assert!(db.index_state(&sb.transcript(ALPHA, S1)).unwrap().is_none());
    // a second scan does not report it again
    assert_eq!(scan(&sb, &mut db).files_missing, 0);
}

#[test]
fn a_new_transcript_and_new_history_rows_show_up() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let new_id = "77777777-aaaa-4bbb-8ccc-000000000007";
    let path = sb.transcript(BETA, new_id);
    std::fs::write(&path, format!(
        "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"brand new\"}},\"uuid\":\"u7\",\"timestamp\":\"2026-09-15T00:00:00.000Z\",\"sessionId\":\"{new_id}\",\"cwd\":\"/Users/me/Code/beta\"}}\n"
    )).unwrap();
    append(&sb.claude.history_file(), &format!(
        "{{\"display\":\"late history row\",\"pastedContents\":{{}},\"timestamp\":{},\"project\":\"/Users/me/Code/delta\",\"sessionId\":\"88888888-aaaa-4bbb-8ccc-000000000008\"}}\n",
        ms("2026-09-16T00:00:00Z")
    ));
    let report = scan(&sb, &mut db);
    assert_eq!(report.files_parsed, 1);
    assert_eq!(db.session(new_id).unwrap().unwrap().first_prompt, "brand new");
    assert_eq!(db.session("88888888-aaaa-4bbb-8ccc-000000000008").unwrap().unwrap().project_key, "-Users-me-Code-delta");
    assert_eq!(db.list_sessions(&ListFilter::default()).unwrap().len(), 10);
}

#[test]
fn schema_version_mismatch_and_corrupt_files_trigger_a_rebuild() {
    let sb = sandbox();
    {
        let mut db = Db::open(&sb.db_path()).unwrap();
        scan(&sb, &mut db);
        db.set_user_version_for_test(0);
    }
    let db = Db::open(&sb.db_path()).unwrap();
    assert!(db.list_sessions(&ListFilter::default()).unwrap().is_empty(), "old schema is dropped");
    drop(db);

    std::fs::write(sb.db_path(), b"this is not a sqlite database").unwrap();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    assert_eq!(db.list_sessions(&ListFilter::default()).unwrap().len(), 8);
}

#[test]
fn rebuild_drops_everything_and_rescans() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    std::fs::remove_file(sb.transcript(ALPHA, S2)).unwrap();
    let report = Indexer::new(sb.claude.clone()).rebuild(&mut db, &mut |_| {}).unwrap();
    assert_eq!(report.files_parsed, 3);
    assert!(db.session(S2).unwrap().is_none(), "a rebuild forgets sessions that have no source left");
    let stats = db.stats().unwrap();
    assert_eq!(stats.sessions, 7);
    assert_eq!(stats.projects, 3);
    assert!(stats.messages > 0);
}

#[test]
fn progress_is_reported_while_scanning() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    let mut seen = Vec::new();
    Indexer::new(sb.claude.clone()).scan(&mut db, &mut |p| seen.push((p.done, p.total))).unwrap();
    assert!(!seen.is_empty());
    assert_eq!(seen.last().unwrap(), &(4, 4));
    assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));
}
