//! Behavior: full-text search over the index (FTS5 trigram, LIKE fallback for short CJK queries).

mod common;

use agent_session_core::index::{Db, Indexer};
use agent_session_core::search::Matcher;
use agent_session_core::search::fts::{FtsQuery, FtsRoles, needs_like, search, search_sessions, to_fts_query};
use common::*;

fn indexed(sb: &Sandbox) -> Db {
    let mut db = Db::open(&sb.db_path()).unwrap();
    Indexer::new(sb.claude.clone(), None).scan(&mut db, &mut |_| {}).unwrap();
    db
}

fn q(text: &str) -> FtsQuery {
    FtsQuery { text: text.into(), roles: FtsRoles::both(), project: None, since_ms: None, sessions: None, limit: 200 }
}

#[test]
fn user_input_is_normalised_into_quoted_phrases() {
    assert_eq!(to_fts_query("hello world"), "\"hello\" \"world\"");
    assert_eq!(to_fts_query("a-b OR c*"), "\"a-b\" \"OR\" \"c*\"", "operators and wildcards are literal");
    assert_eq!(to_fts_query("say \"hi\""), "\"say\" \"\"\"hi\"\"\"");
    assert_eq!(to_fts_query("   "), "");
    assert!(needs_like("重构"), "two CJK chars are shorter than a trigram");
    assert!(needs_like("ab"));
    assert!(!needs_like("解析器"));
    assert!(!needs_like("abc"));
}

#[test]
fn hits_are_grouped_per_session_with_a_count_and_first_snippet() {
    let sb = sandbox();
    let db = indexed(&sb);
    let hits = search_sessions(&db, &q("parser")).unwrap();
    assert_eq!(hits.len(), 1);
    let h = &hits[0];
    assert_eq!(h.session_id, S1);
    assert_eq!(h.project_key, ALPHA);
    assert_eq!(h.count, 3);
    assert!(h.first.text.contains("parser"), "snippet: {:?}", h.first.text);
    assert!(!h.first.ranges.is_empty(), "the snippet carries highlight ranges");
    for r in &h.first.ranges {
        assert_eq!(h.first.text[r.clone()].to_lowercase(), "parser");
    }
    assert_eq!(h.first_ts, chrono::DateTime::parse_from_rfc3339("2026-09-01T08:00:10.000Z").unwrap().timestamp_millis(), "earliest hit in conversation order");
}

#[test]
fn role_project_and_time_filters_apply() {
    let sb = sandbox();
    let db = indexed(&sb);
    let user = search_sessions(&db, &FtsQuery { roles: FtsRoles { user: true, assistant: false }, ..q("parser") }).unwrap();
    assert_eq!(user[0].count, 1);
    let beta = search_sessions(&db, &FtsQuery { project: Some("beta".into()), ..q("parser") }).unwrap();
    assert!(beta.is_empty());
    let late = search_sessions(&db, &FtsQuery { since_ms: Some(chrono::DateTime::parse_from_rfc3339("2026-09-02T00:00:00Z").unwrap().timestamp_millis()), ..q("parser") }).unwrap();
    assert_eq!(late[0].count, 1, "only the 09-02 reply is after the cutoff");
    let none = search_sessions(&db, &FtsQuery { roles: FtsRoles { user: false, assistant: false }, ..q("parser") }).unwrap();
    assert!(none.is_empty());
}

#[test]
fn matching_is_case_insensitive_and_meta_text_is_excluded() {
    let sb = sandbox();
    let db = indexed(&sb);
    assert_eq!(search_sessions(&db, &q("PARSER")).unwrap()[0].count, 3);
    assert!(search_sessions(&db, &q("Caveat")).unwrap().is_empty(), "harness-injected text is not searchable");
    assert!(search_sessions(&db, &q("attachments are never")).unwrap().is_empty());
}

#[test]
fn multi_word_queries_require_every_word() {
    let sb = sandbox();
    let db = indexed(&sb);
    assert_eq!(search_sessions(&db, &q("nested arrays")).unwrap()[0].count, 2);
    assert!(search_sessions(&db, &q("nested friend")).unwrap().is_empty());
    assert!(search_sessions(&db, &q("")).unwrap().is_empty());
}

#[test]
fn cjk_queries_work_for_three_chars_via_fts_and_two_chars_via_like() {
    let sb = sandbox();
    let db = indexed(&sb);
    let three = search_sessions(&db, &q("解析器")).unwrap();
    assert_eq!(three.len(), 1);
    assert_eq!(three[0].session_id, S3);
    assert_eq!(three[0].count, 3);
    let two = search_sessions(&db, &q("谢谢")).unwrap();
    assert_eq!(two.len(), 1);
    assert_eq!(two[0].count, 1);
    assert_eq!(two[0].first.text, "谢谢");
    assert_eq!(two[0].first.ranges, vec![0..6]);
    let two_user_only = search_sessions(&db, &FtsQuery { roles: FtsRoles { user: false, assistant: true }, ..q("重构") }).unwrap();
    assert_eq!(two_user_only[0].count, 1, "LIKE path honours the role filter");
}

#[test]
fn sessions_are_ordered_newest_first_and_limited() {
    let sb = sandbox();
    let db = indexed(&sb);
    let hits = search_sessions(&db, &q("the")).unwrap();
    let ids: Vec<&str> = hits.iter().map(|h| h.session_id.as_str()).collect();
    assert_eq!(ids, vec![S2, S1]);
    let one = search_sessions(&db, &FtsQuery { limit: 1, ..q("the") }).unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].session_id, S2);
}

#[test]
fn messages_deleted_with_their_transcript_disappear_from_search() {
    let sb = sandbox();
    let mut db = indexed(&sb);
    std::fs::remove_file(sb.transcript(ALPHA, S1)).unwrap();
    Indexer::new(sb.claude.clone(), None).scan(&mut db, &mut |_| {}).unwrap();
    assert!(search_sessions(&db, &q("parser")).unwrap().is_empty());
}

#[test]
fn the_outcome_reports_totals_beyond_the_limit_and_honours_a_session_filter() {
    let sb = sandbox();
    let db = indexed(&sb);
    let all = search(&db, &q("the")).unwrap();
    assert_eq!((all.hits.len(), all.total_sessions), (2, 2));
    assert_eq!(all.total_matches, all.hits.iter().map(|h| h.count as usize).sum::<usize>());

    let one = search(&db, &FtsQuery { limit: 1, ..q("the") }).unwrap();
    assert_eq!(one.hits.len(), 1, "only the newest session is kept");
    assert_eq!(one.total_sessions, 2, "but the caller learns there were more");
    assert_eq!(one.total_matches, all.total_matches);

    let only_s1 = search(&db, &FtsQuery { sessions: Some([S1.to_string()].into()), ..q("the") }).unwrap();
    assert_eq!(only_s1.hits.iter().map(|h| h.session_id.as_str()).collect::<Vec<_>>(), [S1]);
    assert_eq!(only_s1.total_sessions, 1, "sessions outside the filter do not count");
}

#[test]
fn snippets_highlight_every_query_word_around_the_first_one() {
    let sb = sandbox();
    let db = indexed(&sb);
    let hits = search_sessions(&db, &q("arrays nested")).unwrap();
    let first = &hits[0].first;
    let marked: Vec<String> = first.ranges.iter().map(|r| first.text[r.clone()].to_lowercase()).collect();
    assert!(marked.contains(&"nested".to_string()) && marked.contains(&"arrays".to_string()), "marked: {marked:?} in {:?}", first.text);
    assert!(!first.text.contains('\n'), "snippets are one line");
}

#[test]
fn a_matcher_marks_words_or_a_regex_in_any_text() {
    let words = Matcher::words("Parser  plan").unwrap();
    assert_eq!(words.ranges("the PLAN: refactor the parser"), vec![4..8, 23..29]);
    assert!(words.is_match("planning") && !words.is_match("nothing"));
    assert!(Matcher::words("   ").is_none(), "a blank query matches nothing");
    assert_eq!(Matcher::words("a.b").unwrap().ranges("axb a.b"), vec![4..7], "words are literal text");

    let re = Matcher::regex(r"pars\w+", false);
    assert_eq!(re.ranges("Parser parsing"), vec![0..6, 7..14]);
    assert!(Matcher::regex("Parser", true).ranges("parser").is_empty());
    assert_eq!(Matcher::regex("a(", false).ranges("f(a(b))"), vec![2..4], "an invalid regex is literal text");
    assert!(Matcher::regex("x*", false).ranges("abc").is_empty(), "empty matches are not highlights");

    let cut = words.snippet(&format!("{}\nthe parser{}", "a".repeat(100), "b".repeat(100)), 10, 20);
    assert_eq!(cut.text, format!("…{}⏎the parser{}…", "a".repeat(5), "b".repeat(14)));
    assert_eq!(&cut.text[cut.ranges[0].clone()], "parser");
}
