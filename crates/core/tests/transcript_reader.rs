//! Behavior contract for compact session handoffs.
mod common;
use agent_session_core::transcript::{read_markdown, resolve_transcript};
use assert_cmd::Command;
use common::*;

#[test]
fn exports_human_inputs_final_answers_and_tool_counts_without_intermediate_noise() {
    let path = fixture_root().parent().unwrap().join("reader.jsonl");
    let export = read_markdown(&path).unwrap();
    assert_eq!(export.markdown, "# Session reader\n\n## User\n\n修复问题\n\n## Assistant\n\n> Tools: Bash ×2, Read ×1\n\n完成。\n\n```rust\nlet x = 1;\n```\n\n## User\n\n继续\n\n## Assistant\n\n> Tools: Edit ×1\n\n*[No final response recorded]*\n\n## User\n\n[Image omitted]\n\n## Assistant\n\n这是一张图片。\n\n");
    assert_eq!(export.skipped_lines, 1);
}

#[test]
fn resolves_exact_ids_unique_prefixes_and_paths_without_creating_an_index() {
    let sb = sandbox();
    assert_eq!(resolve_transcript(&sb.claude, S1).unwrap(), sb.transcript(ALPHA, S1));
    assert_eq!(resolve_transcript(&sb.claude, "1111").unwrap(), sb.transcript(ALPHA, S1));
    let path = sb.transcript(ALPHA, S2);
    assert_eq!(resolve_transcript(&sb.claude, path.to_str().unwrap()).unwrap(), path);
    std::fs::copy(sb.transcript(ALPHA, S1), sb.transcript(ALPHA, "1111-other")).unwrap();
    assert!(resolve_transcript(&sb.claude, "1111").unwrap_err().to_string().contains("ambiguous"));
    assert!(resolve_transcript(&sb.claude, S4).unwrap_err().to_string().contains("no transcript"));
    assert!(!sb.db_path().exists());
}

#[test]
fn both_cli_entrypoints_export_identical_markdown_and_refuse_overwriting_files() {
    let sb = sandbox();
    let standalone = Command::cargo_bin("transcript-reader").unwrap()
        .env("ASM_CLAUDE_DIR", sb.root()).arg(S2).output().unwrap();
    assert!(standalone.status.success(), "{}", String::from_utf8_lossy(&standalone.stderr));
    let integrated = Command::cargo_bin("asm").unwrap()
        .env("ASM_CLAUDE_DIR", sb.root()).args(["transcript", S2]).output().unwrap();
    assert!(integrated.status.success());
    assert_eq!(standalone.stdout, integrated.stdout);
    let output = sb.tmp.path().join("session.md");
    Command::cargo_bin("transcript-reader").unwrap().env("ASM_CLAUDE_DIR", sb.root())
        .arg(S2).arg("-o").arg(&output).assert().success().stdout("");
    assert_eq!(std::fs::read(&output).unwrap(), standalone.stdout);
    Command::cargo_bin("transcript-reader").unwrap().env("ASM_CLAUDE_DIR", sb.root())
        .arg(S1).arg("-o").arg(&output).assert().failure();
    assert_eq!(std::fs::read(&output).unwrap(), standalone.stdout);
}

#[test]
fn accepts_valid_unterminated_last_record_but_reports_a_truncated_record() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("live.jsonl");
    std::fs::write(&path, r#"{"type":"user","message":{"content":"hello"}}"#).unwrap();
    let export = read_markdown(&path).unwrap();
    assert!(export.markdown.contains("hello"));
    assert_eq!(export.skipped_lines, 0);
    append(&path, "\n{\"type\":");
    assert_eq!(read_markdown(&path).unwrap().skipped_lines, 1);
}

#[test]
fn keeps_completed_turns_before_automatic_continuation_and_marks_aborted_text() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("turns.jsonl");
    std::fs::write(&path, concat!(
        "{\"type\":\"assistant\",\"message\":{\"id\":\"a\",\"stop_reason\":\"end_turn\",\"content\":\"First final\"}}\n",
        "{\"type\":\"assistant\",\"message\":{\"id\":\"b\",\"stop_reason\":\"tool_use\",\"content\":[{\"type\":\"tool_use\",\"name\":\"Read\"}]}}\n",
        "{\"type\":\"assistant\",\"isAbortedMidStream\":true,\"message\":{\"id\":\"c\",\"content\":\"Unfinished text\"}}\n"
    )).unwrap();
    let export = read_markdown(&path).unwrap();
    assert!(export.markdown.contains("First final"));
    assert!(export.markdown.contains("[No final response recorded]"));
    assert!(!export.markdown.contains("Unfinished text"));
}

#[test]
fn explicit_subagent_path_keeps_its_dialogue() {
    let sb = sandbox();
    let path = sb.transcript(ALPHA, S1).with_extension("").join("subagents/agent-a1b2c3.jsonl");
    let export = read_markdown(&path).unwrap();
    assert!(export.markdown.contains("## User"));
    assert!(export.markdown.contains("## Assistant"));
}
