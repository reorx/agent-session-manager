//! Behavior: the SQLite index reflects the transcripts + history on disk, and rescans only touch
//! what changed (append / rewrite / delete / new file).

mod common;

use agent_session_core::claude::parse::parse_transcript;
use agent_session_core::index::{Db, Indexer, ListFilter, Order, SCHEMA_VERSION, ScanEvent};
use agent_session_core::model::{Role, SessionSummary, Source};
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
    assert_eq!((report.subagents_seen, report.subagents_parsed), (1, 1), "subagents are scanned for their cost");
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
    assert_eq!((s4.cost_usd, s4.tokens_in, s4.context_tokens), (None, 0, None), "history knows nothing of usage");
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
    assert_close(s1.cost_usd, HAIKU_REPLY + SUBAGENT, "the old main usage is gone, the subagent's stays");
    assert_eq!(s1.context_model.as_deref(), Some("claude-haiku-4-5-20251001"));
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
    assert_close(s1.cost_usd, S1_MAIN + SUBAGENT, "and so is what it cost");
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
        db.set_user_version_for_test(SCHEMA_VERSION - 1);
    }
    let db = Db::open(&sb.db_path()).unwrap();
    assert!(db.list_sessions(&ListFilter::default()).unwrap().is_empty(), "old schema is dropped");
    assert!(db.index_state(&sb.transcript(ALPHA, S1)).unwrap().is_none(), "so every file is parsed again");
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

/// Scan and collect every session streamed out while the index is being written.
fn scan_streaming(sb: &Sandbox, db: &mut Db) -> Vec<SessionSummary> {
    let mut streamed = Vec::new();
    Indexer::new(sb.claude.clone())
        .scan_streaming(db, &mut |ev| {
            if let ScanEvent::Session(s) = ev {
                streamed.push(s);
            }
        })
        .unwrap();
    streamed
}

#[test]
fn a_cold_scan_streams_every_session_exactly_as_the_index_stores_it() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    let streamed = scan_streaming(&sb, &mut db);
    assert_eq!(streamed.len(), 8, "4 transcripts + 4 history-only sessions");
    for s in &streamed {
        let mut indexed = db.session(&s.id).unwrap().unwrap();
        if s.project_path.is_none() {
            // S5 has no cwd: its project's real path is only learned from a session written later
            indexed.project_path = None;
        }
        assert_eq!(s, &indexed, "streamed row for {} matches the index", s.id);
    }
    let s1 = streamed.iter().find(|s| s.id == S1).unwrap();
    assert_eq!(s1.project_path.as_deref(), Some("/Users/me/Code/alpha"), "rows are usable by the UI without a reload");
}

#[test]
fn a_rescan_streams_only_the_sessions_that_changed() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan_streaming(&sb, &mut db);
    assert!(scan_streaming(&sb, &mut db).is_empty(), "nothing changed on disk, history included");

    let path = sb.transcript(ALPHA, S2);
    append(&path, concat!(
        r#"{"parentUuid":"x","isSidechain":false,"type":"user","message":{"role":"user","content":"And now in French"},"uuid":"u-new","timestamp":"2026-09-06T00:00:00.000Z","sessionId":"22222222-aaaa-4bbb-8ccc-000000000002","cwd":"/Users/me/Code/alpha","gitBranch":"main","entrypoint":"sdk-cli","version":"2.1.261"}"#, "\n",
    ));
    set_mtime(&path, MTIME_S2 + 60_000);
    let streamed = scan_streaming(&sb, &mut db);
    assert_eq!(streamed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [S2]);
    assert_eq!(streamed[0].last_prompt.as_deref(), Some("And now in French"));
    assert_eq!(streamed[0].updated_at, MTIME_S2 + 60_000);

    std::fs::remove_file(sb.transcript(ALPHA, S1)).unwrap();
    let streamed = scan_streaming(&sb, &mut db);
    assert_eq!(streamed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [S1]);
    assert!(streamed[0].transcript_missing, "the UI learns that the row turned history-only");
}

#[test]
fn a_new_history_row_is_streamed_once() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan_streaming(&sb, &mut db);
    let new_id = "88888888-aaaa-4bbb-8ccc-000000000008";
    append(&sb.claude.history_file(), &format!(
        "{{\"display\":\"late history row\",\"pastedContents\":{{}},\"timestamp\":{},\"project\":\"/Users/me/Code/delta\",\"sessionId\":\"{new_id}\"}}\n",
        ms("2026-09-16T00:00:00Z")
    ));
    let streamed = scan_streaming(&sb, &mut db);
    assert_eq!(streamed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [new_id]);
    assert_eq!(streamed[0].source, Source::History);
    assert!(scan_streaming(&sb, &mut db).is_empty());
}

#[test]
fn a_read_only_connection_searches_while_the_writer_keeps_the_index() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    Indexer::new(sb.claude.clone()).scan(&mut db, &mut |_| {}).unwrap();
    let reader = Db::open_read_only(&sb.db_path()).unwrap();
    assert_eq!(reader.list_sessions(&Default::default()).unwrap().len(), db.list_sessions(&Default::default()).unwrap().len());
    assert!(reader.conn().execute("DELETE FROM sessions", []).is_err(), "the reader cannot write");
    assert!(Db::open_read_only(&sb.tmp.path().join("missing.sqlite")).is_err(), "and never creates an index");
}

/// Fixture costs: S1's own five API responses (Opus 5, one in fast mode), its subagent's single
/// Sonnet 5 reply, and the Haiku 4.5 reply of S2.
const S1_MAIN: f64 = 0.12194;
const SUBAGENT: f64 = (50.0 * 2.0 + 1000.0 * 2.5 + 500.0 * 10.0) / 1e6;
const HAIKU_REPLY: f64 = (10.0 * 1.0 + 20.0 * 5.0) / 1e6;

fn assert_close(got: Option<f64>, want: f64, what: &str) {
    let got = got.unwrap_or_else(|| panic!("{what}: no cost"));
    assert!((got - want).abs() < 1e-9, "{what}: {got} != {want}");
}

fn subagent_path(sb: &Sandbox) -> std::path::PathBuf {
    sb.root().join("projects").join(ALPHA).join(S1).join("subagents/agent-a1b2c3.jsonl")
}

fn reply(msg_id: &str, model: &str, input: u64, output: u64, sidechain: bool) -> String {
    format!(
        "{}\n",
        serde_json::json!({
            "type": "assistant", "isSidechain": sidechain, "uuid": format!("u-{msg_id}-{input}"),
            "timestamp": "2026-09-06T00:00:00.000Z", "requestId": format!("req-{msg_id}"),
            "message": {"id": msg_id, "model": model, "role": "assistant", "content": [{"type": "text", "text": "ok"}],
                        "usage": {"input_tokens": input, "output_tokens": output}},
        })
    )
}

#[test]
fn sessions_carry_their_cost_tokens_and_context_with_subagents_counted_in() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let s1 = db.session(S1).unwrap().unwrap();
    assert_close(s1.cost_usd, S1_MAIN + SUBAGENT, "S1");
    assert_eq!((s1.tokens_in, s1.tokens_out, s1.tokens_cache_read, s1.tokens_cache_write), (12 + 50, 1050 + 500, 89_000, 2900 + 1000));
    assert_eq!((s1.context_tokens, s1.context_peak), (Some(14_305), Some(40_001)), "the subagent's calls are not the session's context");
    assert_eq!(s1.context_model.as_deref(), Some("claude-opus-5"));
    assert_eq!(s1.total_cost_usd, Some(1.2345), "Claude Code's own figure is still there");
    assert!(s1.unpriced_models.is_empty());
    assert_close(db.session(S2).unwrap().unwrap().cost_usd, HAIKU_REPLY, "S2 (its <synthetic> record is free)");
    assert_eq!(db.session(S5).unwrap().unwrap().cost_usd, None, "a session without any API call has no cost");
}

#[test]
fn a_response_cut_between_two_scans_is_counted_once() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let path = sb.transcript(ALPHA, S2);
    append(&path, &reply("msg-cut", "claude-haiku-4-5-20251001", 100, 10, false));
    set_mtime(&path, MTIME_S2 + 1_000);
    scan(&sb, &mut db);
    append(&path, &reply("msg-cut", "claude-haiku-4-5-20251001", 100, 10, false));
    set_mtime(&path, MTIME_S2 + 2_000);
    let report = scan(&sb, &mut db);
    assert_eq!(report.files_appended, 1);
    let s2 = db.session(S2).unwrap().unwrap();
    let full = parse_transcript(&path).unwrap().summary;
    assert_close(s2.cost_usd, full.cost_usd.unwrap(), "incremental == full parse");
    assert_close(s2.cost_usd, HAIKU_REPLY + (100.0 + 10.0 * 5.0) / 1e6, "the second half adds nothing");
    assert_eq!(s2.tokens_in, 10 + 100);
    assert_eq!((s2.context_tokens, s2.context_peak), (Some(100), Some(100)));
}

#[test]
fn a_subagent_that_grows_raises_its_parents_cost_and_streams_the_parent_again() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan_streaming(&sb, &mut db);
    append(&subagent_path(&sb), &reply("msg-sub-2", "claude-sonnet-5", 1_000_000, 0, true));
    let mut streamed = Vec::new();
    let report = Indexer::new(sb.claude.clone())
        .scan_streaming(&mut db, &mut |ev| {
            if let ScanEvent::Session(s) = ev {
                streamed.push(s);
            }
        })
        .unwrap();
    assert_eq!((report.files_appended, report.subagents_parsed), (0, 1), "only the subagent file changed");
    assert_eq!(streamed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [S1], "the parent row is sent once");
    assert_close(streamed[0].cost_usd, S1_MAIN + SUBAGENT + 2.0, "a million Sonnet 5 input tokens");
    assert_eq!(streamed[0].context_tokens, Some(14_305), "the context is still the main thread's");
    assert!(scan_streaming(&sb, &mut db).is_empty(), "and the next scan is quiet again");
}

#[test]
fn a_deleted_subagent_takes_its_cost_along_but_leaves_the_parent_alone() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    std::fs::remove_file(subagent_path(&sb)).unwrap();
    let streamed = scan_streaming(&sb, &mut db);
    assert_eq!(streamed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [S1]);
    let s1 = db.session(S1).unwrap().unwrap();
    assert_close(s1.cost_usd, S1_MAIN, "S1 without its subagent");
    assert!(!s1.transcript_missing, "the session's own transcript is still there");
    assert!(db.index_state(&subagent_path(&sb)).unwrap().is_none());
    assert!(scan_streaming(&sb, &mut db).is_empty());
}

#[test]
fn a_new_subagent_of_an_unchanged_session_is_picked_up() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let dir = sb.root().join("projects").join(BETA).join(S3).join("subagents");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("agent-zz.jsonl"), reply("msg-zz", "claude-opus-5", 1_000_000, 0, true)).unwrap();
    let report = scan(&sb, &mut db);
    assert_eq!((report.subagents_seen, report.subagents_parsed), (2, 1));
    let s3 = db.session(S3).unwrap().unwrap();
    assert_close(s3.cost_usd, 2.0 * (10.0 * 5.0 + 20.0 * 25.0) / 1e6 + 5.0, "two own replies + a million Opus 5 input tokens");
}

#[test]
fn rebuild_clears_the_usage_too() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let before = db.session(S1).unwrap().unwrap().cost_usd;
    Indexer::new(sb.claude.clone()).rebuild(&mut db, &mut |_| {}).unwrap();
    assert_eq!(db.session(S1).unwrap().unwrap().cost_usd, before, "a rebuild counts every response exactly once again");
}

#[test]
fn a_cleaned_up_session_keeps_what_its_subagents_cost() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    std::fs::remove_file(sb.transcript(ALPHA, S1)).unwrap();
    std::fs::remove_dir_all(sb.root().join("projects").join(ALPHA).join(S1)).unwrap();
    let streamed = scan_streaming(&sb, &mut db);
    assert_eq!(streamed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [S1], "sent once, as history only");
    assert!(streamed[0].transcript_missing);
    assert_close(streamed[0].cost_usd, S1_MAIN + SUBAGENT, "Claude Code removed both files; the money was still spent");
}
