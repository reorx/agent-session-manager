//! Behavior: ~/.claude/history.jsonl is grouped into sessions exactly like the Python reference
//! (`load_history` / `history_sessions` / `expand_pasted`).

mod common;

use agent_session_core::claude::history::{expand_pasted, history_prompts, history_sessions, load_history};
use agent_session_core::claude::paths::encode_project_path;
use agent_session_core::model::Source;
use common::*;

fn ms(iso: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(iso).unwrap().timestamp_millis()
}

#[test]
fn project_paths_are_encoded_like_claude_code_does() {
    assert_eq!(encode_project_path("/Users/reorx/Code/scripts"), "-Users-reorx-Code-scripts");
    assert_eq!(encode_project_path("/Users/me/Code/campus_watch"), "-Users-me-Code-campus-watch");
    assert_eq!(encode_project_path("/tmp/a.b/c d/é"), "-tmp-a-b-c-d--");
}

#[test]
fn rows_are_grouped_by_encoded_project_in_file_order_and_junk_is_skipped() {
    let sb = sandbox();
    let by_project = load_history(&sb.claude.history_file()).unwrap();
    let mut keys: Vec<_> = by_project.keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, vec![ALPHA, BETA, GAMMA]);
    assert_eq!(by_project[ALPHA].len(), 7);
    assert_eq!(by_project[BETA].len(), 4);
    assert_eq!(by_project[GAMMA][0].display, "hello gamma");
    assert!(by_project[ALPHA].windows(2).all(|w| w[0].timestamp <= w[1].timestamp), "chronological");
}

#[test]
fn rows_with_session_id_form_one_session_with_pasted_text_expanded() {
    let sb = sandbox();
    let by_project = load_history(&sb.claude.history_file()).unwrap();
    let sessions = history_sessions(ALPHA, &by_project[ALPHA], &sb.claude.paste_cache_dir());
    let s4 = sessions.iter().find(|s| s.id == S4).expect("deleted transcript recovered from history");
    assert_eq!(s4.source, Source::History);
    assert!(s4.id_known);
    assert_eq!(s4.project_key, ALPHA);
    assert_eq!(s4.project_path.as_deref(), Some("/Users/me/Code/alpha"));
    assert_eq!(s4.user_msg_count, 4, "every history row counts as a prompt, slash commands included");
    assert_eq!(s4.started_at, Some(ms("2026-08-20T01:00:00Z")), "started at the first row even if it is a command");
    assert_eq!(s4.updated_at, ms("2026-08-20T02:30:00Z"));
    assert_eq!(s4.first_prompt, "Fix the flaky test\n\nline1\nline2\nline3", "first non-command row, inline paste expanded");
    assert_eq!(s4.last_prompt.as_deref(), Some("[Pasted text #3 +9 lines]"), "unresolvable paste placeholder is kept");
    assert_eq!(s4.file_size, None);
    assert_eq!(s4.file_path, None);

    let s1 = sessions.iter().find(|s| s.id == S1).unwrap();
    assert_eq!(s1.user_msg_count, 3);
    assert_eq!(s1.first_prompt, "Please refactor the parser to handle nested arrays");
}

#[test]
fn pasted_text_comes_from_inline_content_or_the_paste_cache() {
    let sb = sandbox();
    let cache = sb.claude.paste_cache_dir();
    let inline = serde_json::json!({"1": {"id": 1, "type": "text", "content": "A\nB"}});
    assert_eq!(expand_pasted("x [Pasted text #1 +1 lines] y", inline.as_object().unwrap(), &cache), "x A\nB y");
    let cached = serde_json::json!({"2": {"id": 2, "type": "text", "contentHash": "deadbeef01234567"}});
    assert_eq!(expand_pasted("[Pasted text #2] and explain", cached.as_object().unwrap(), &cache), "cached paste and explain");
    let missing = serde_json::json!({"3": {"id": 3, "type": "text", "contentHash": "0000missinghash0"}});
    assert_eq!(expand_pasted("[Pasted text #3 +9 lines]", missing.as_object().unwrap(), &cache), "[Pasted text #3 +9 lines]");
    let empty = serde_json::Map::new();
    assert_eq!(expand_pasted("[Pasted text #9]", &empty, &cache), "[Pasted text #9]");
}

#[test]
fn rows_without_session_id_are_split_into_anonymous_sessions_on_a_two_hour_gap() {
    let sb = sandbox();
    let by_project = load_history(&sb.claude.history_file()).unwrap();
    let sessions = history_sessions(BETA, &by_project[BETA], &sb.claude.paste_cache_dir());
    assert_eq!(sessions.len(), 2);
    let (a, b) = (&sessions[0], &sessions[1]);
    assert!(!a.id_known && !b.id_known);
    assert_ne!(a.id, b.id, "anonymous sessions still get distinct stable ids");
    assert_eq!(a.first_prompt, "design a plagiarism checker");
    assert_eq!(a.user_msg_count, 2);
    assert_eq!(a.started_at, Some(ms("2025-06-01T00:00:00Z")));
    assert_eq!(a.updated_at, ms("2025-06-01T00:30:00Z"));
    assert_eq!(b.first_prompt, "now add a CLI", "first row that is not a slash command");
    assert_eq!(b.started_at, Some(ms("2025-06-01T05:30:00Z")), "but the session starts at the /clear row");
    assert_eq!(b.user_msg_count, 2);
}

#[test]
fn a_session_made_only_of_commands_uses_the_first_row() {
    let rows = vec![
        agent_session_core::claude::history::HistoryRow { display: "/clear".into(), pasted: Default::default(), timestamp: 1_000, project: "/p".into(), session_id: Some("sid".into()) },
        agent_session_core::claude::history::HistoryRow { display: "/model opus".into(), pasted: Default::default(), timestamp: 2_000, project: "/p".into(), session_id: Some("sid".into()) },
    ];
    let sessions = history_sessions("-p", &rows, std::path::Path::new("/nonexistent"));
    assert_eq!(sessions[0].first_prompt, "/clear");
    assert_eq!(sessions[0].user_msg_count, 2);
}

#[test]
fn history_prompts_of_a_session_are_its_rows_with_pasted_text_restored() {
    let sb = sandbox();
    let prompts = history_prompts(&sb.claude, ALPHA, S4).unwrap();
    assert_eq!(prompts.len(), 4);
    assert_eq!(prompts[0].text, "/model opus");
    assert_eq!(prompts[0].ts, ms("2026-08-20T01:00:00Z"));
    assert_eq!(prompts[1].text, "Fix the flaky test\n\nline1\nline2\nline3");
    assert!(prompts[2].text.contains("cached paste"));
    assert_eq!(prompts[3].text, "[Pasted text #3 +9 lines]");
    assert!(history_prompts(&sb.claude, BETA, S4).unwrap().is_empty());
    assert!(history_prompts(&sb.claude, "missing-project", S4).unwrap().is_empty());
}

#[test]
fn an_anonymous_history_session_gets_the_prompts_of_its_time_group() {
    let sb = sandbox();
    let id = format!("history:{BETA}:{}", ms("2025-06-01T05:30:00Z"));
    let prompts = history_prompts(&sb.claude, BETA, &id).unwrap();
    assert_eq!(prompts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["/clear", "now add a CLI"]);
    assert_eq!(prompts[0].ts, ms("2025-06-01T05:30:00Z"));
}

#[test]
fn history_sessions_and_history_prompts_group_the_same_way() {
    let sb = sandbox();
    // Exactly two hours stays in the anonymous group, even across a known-id row.
    append(&sb.claude.history_file(), concat!(
        "{\"display\":\"first\",\"timestamp\":0,\"project\":\"/mixed\"}\n",
        "{\"display\":\"known\",\"timestamp\":1,\"project\":\"/mixed\",\"sessionId\":\"known\"}\n",
        "{\"display\":\"\",\"timestamp\":7200000,\"project\":\"/mixed\",\"sessionId\":\"\"}\n",
        "{\"display\":\"next\",\"timestamp\":14400001,\"project\":\"/mixed\"}\n"
    ));
    let projects = load_history(&sb.claude.history_file()).unwrap();
    for (key, rows) in projects {
        let sessions = history_sessions(&key, &rows, &sb.claude.paste_cache_dir());
        if key == "-mixed" {
            assert_eq!(sessions.iter().map(|s| s.user_msg_count).collect::<Vec<_>>(), [2, 1, 1]);
        }
        for session in sessions {
            let prompts = history_prompts(&sb.claude, &key, &session.id).unwrap();
            assert_eq!(prompts.len(), session.user_msg_count as usize);
            assert_eq!(Some(prompts[0].ts), session.started_at);
            assert_eq!(prompts.last().unwrap().ts, session.updated_at);
            assert_eq!(Some(&prompts.last().unwrap().text), session.last_prompt.as_ref());
        }
    }
}
