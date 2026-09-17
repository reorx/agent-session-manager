//! Behavior: regex search over raw transcripts reproduces the Python reference's search mode
//! (`iter_texts` / `search_file` / `make_snippet` / `render_tool_use`).

mod common;

use agent_session_core::search::regex::{
    RegexOptions, RoleSet, Snippet, ToolLineKind, compile_query, list_transcripts, search,
};
use common::*;

fn opts() -> RegexOptions {
    RegexOptions { roles: RoleSet::all(), labels: vec![], case_sensitive: false, context: 80, limit: 200 }
}

fn run(sb: &Sandbox, query: &str, o: RegexOptions) -> agent_session_core::search::regex::SearchOutcome {
    let files = list_transcripts(&sb.claude, None, None).unwrap();
    search(query, &files, &o)
}

#[test]
fn transcripts_are_listed_newest_first_with_day_and_project_filters() {
    let sb = sandbox();
    let files = list_transcripts(&sb.claude, None, None).unwrap();
    let ids: Vec<&str> = files.iter().map(|f| f.session_id.as_str()).collect();
    assert_eq!(ids, vec![S3, S2, S5, S1], "mtime desc; subagent files are never scanned");
    assert_eq!(files[0].project_key, BETA);

    let beta = list_transcripts(&sb.claude, None, Some("BETA")).unwrap();
    assert_eq!(beta.len(), 1, "project substring is case-insensitive");

    let now = chrono::Utc::now().timestamp_millis();
    let days_since_s3 = (now - MTIME_S3) as f64 / 86_400_000.0;
    let recent = list_transcripts(&sb.claude, Some(days_since_s3 + 0.01), None).unwrap();
    assert_eq!(recent.len(), 1);
}

#[test]
fn a_match_is_counted_once_per_message_piece_in_conversation_order() {
    let sb = sandbox();
    let out = run(&sb, "parser", opts());
    assert_eq!(out.scanned, 4);
    assert_eq!(out.total, 3);
    assert_eq!(out.files.len(), 1);
    let f = &out.files[0];
    assert_eq!(f.session_id, S1);
    assert_eq!(f.mtime_ms, MTIME_S1);
    let roles: Vec<_> = f.matches.iter().map(|m| m.role).collect();
    use agent_session_core::search::regex::MatchRole::*;
    assert_eq!(roles, vec![Assistant, User, Assistant]);
    let first = &f.matches[0];
    assert_eq!(first.timestamp_ms, Some(chrono::DateTime::parse_from_rfc3339("2026-09-01T08:00:10.000Z").unwrap().timestamp_millis()));
    match &first.snippet {
        Snippet::Text(h) => {
            assert_eq!(h.text, "Here is the plan: refactor the parser.");
            assert_eq!(h.ranges, vec![31..37]);
        }
        other => panic!("expected text snippet, got {other:?}"),
    }
}

#[test]
fn role_and_label_filters_select_pieces() {
    let sb = sandbox();
    let user_only = run(&sb, "parser", RegexOptions { roles: RoleSet { user: true, assistant: false, tool: false }, ..opts() });
    assert_eq!(user_only.total, 1);

    let bash = run(&sb, "cargo", RegexOptions { labels: vec!["tool_use:Bash".into()], ..opts() });
    assert_eq!(bash.total, 1);
    let m = &bash.files[0].matches[0];
    assert_eq!(m.detail, "tool_use:Bash");
    match &m.snippet {
        Snippet::ToolUse(lines) => {
            assert_eq!(lines[0].kind, ToolLineKind::Description);
            assert_eq!(lines[0].text.text, "Run the tests");
            assert_eq!(lines[1].kind, ToolLineKind::Command);
            assert_eq!(lines[1].text.text, "cargo test -p alpha 2>&1 | tail -5");
            assert_eq!(lines[1].text.ranges, vec![0..5]);
            assert_eq!(lines[2].kind, ToolLineKind::Field("timeout".into()));
            assert_eq!(lines[2].text.text, "120000", "non-string values are JSON encoded");
        }
        other => panic!("expected tool_use snippet, got {other:?}"),
    }

    let thinking = run(&sb, "terminal", RegexOptions { labels: vec!["thinking".into()], ..opts() });
    assert_eq!(thinking.total, 1);
    assert_eq!(thinking.files[0].matches[0].detail, "thinking");

    let results = run(&sb, "passed", RegexOptions { labels: vec!["tool_result".into()], ..opts() });
    assert_eq!(results.total, 1);

    let any_tool = run(&sb, "src", RegexOptions { labels: vec!["tool".into()], ..opts() });
    assert_eq!(any_tool.total, 3, "'tool' matches tool_use and tool_result labels: Bash output, Grep input, Grep output");

    let labelled_user = run(&sb, "parser", RegexOptions { labels: vec!["tool".into()], roles: RoleSet { user: true, assistant: false, tool: false }, ..opts() });
    assert_eq!(labelled_user.total, 0, "prompts have no label, any -l leaves them out");
}

#[test]
fn case_sensitivity_and_literal_fallback() {
    let sb = sandbox();
    assert_eq!(run(&sb, "Parser", opts()).total, 3, "case-insensitive by default");
    assert_eq!(run(&sb, "Parser", RegexOptions { case_sensitive: true, ..opts() }).total, 0);
    assert_eq!(run(&sb, "[Image", opts()).total, 1, "invalid regex is searched literally");
    assert_eq!(run(&sb, "refactor the (parser|world)", opts()).total, 2, "valid regex is a regex");
    assert!(compile_query("(unclosed", false).is_ok());
}

#[test]
fn chinese_text_is_matched_and_snippets_are_cut_in_characters() {
    let sb = sandbox();
    let out = run(&sb, "解析器", opts());
    assert_eq!(out.total, 3);
    let out = run(&sb, "解析器", RegexOptions { context: 2, ..opts() });
    match &out.files[0].matches[0].snippet {
        Snippet::Text(h) => {
            assert_eq!(h.text, "…一下解析器，处…");
            assert_eq!(h.ranges, vec!["…一下".len().."…一下解析器".len()]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn snippet_context_marks_cuts_and_line_breaks() {
    let sb = sandbox();
    let out = run(&sb, "nested", RegexOptions { context: 5, roles: RoleSet { user: true, assistant: false, tool: false }, ..opts() });
    match &out.files[0].matches[0].snippet {
        Snippet::Text(h) => {
            assert_eq!(h.text, "…ndle nested arra…");
            assert_eq!(h.ranges, vec![8..14]);
        }
        other => panic!("{other:?}"),
    }
    let out = run(&sb, "nice", RegexOptions { context: 4, ..opts() });
    match &out.files[0].matches[0].snippet {
        Snippet::Text(h) => assert_eq!(h.text, "… be nice⏎</s…"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn long_tool_fields_are_cut_around_the_match() {
    let long = "x".repeat(400);
    let line = format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t","name":"Write","input":{{"file_path":"/tmp/a","content":"{long}NEEDLE{long}"}}}}]}},"uuid":"a","timestamp":"2026-01-01T00:00:00.000Z"}}"#
    );
    let out = agent_session_core::search::regex::search_bytes(line.as_bytes(), &compile_query("needle", false).unwrap(), &opts());
    assert_eq!(out.len(), 1);
    match &out[0].snippet {
        Snippet::ToolUse(lines) => {
            let content = lines.iter().find(|l| l.kind == ToolLineKind::Field("content".into())).unwrap();
            assert_eq!(content.text.text.chars().count(), 80 + 6 + 80 + 2);
            assert!(content.text.text.starts_with('…') && content.text.text.ends_with('…'));
            let path = lines.iter().find(|l| l.kind == ToolLineKind::Field("file_path".into())).unwrap();
            assert_eq!(path.text.text, "/tmp/a");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn limit_stops_after_the_file_that_reaches_it() {
    let sb = sandbox();
    let out = run(&sb, "e", RegexOptions { limit: 1, ..opts() });
    assert!(out.total >= 1);
    assert_eq!(out.shown, 1);
    assert_eq!(out.files[0].session_id, S3, "newest file first");
    assert_eq!(out.files[0].matches.len(), 1);
    assert!(out.files[0].match_count > 1, "the header still reports every match of the file");
    assert!(out.files.len() <= 1, "later files are not scanned once the limit is reached");
}
