//! Behavior: the `asm` CLI prints what `claude_session_search.py` prints (colors stripped).

mod common;

use assert_cmd::Command;
use common::*;

fn asm(sb: &Sandbox) -> Command {
    let mut cmd = Command::cargo_bin("asm").unwrap();
    cmd.env("ASM_CLAUDE_DIR", sb.root())
        .env("ASM_DB", sb.db_path())
        .env("TZ", "UTC")
        .env("NO_COLOR", "1")
        .env_remove("CLAUDE_CONFIG_DIR");
    cmd
}

fn size(sb: &Sandbox, project: &str, sid: &str) -> String {
    agent_session_core::format::human_size(std::fs::metadata(sb.transcript(project, sid)).unwrap().len())
}

#[test]
fn list_prints_a_projects_sessions_newest_first_like_the_reference() {
    let sb = sandbox();
    let out = asm(&sb).args(["list", "alpha"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let expected = format!(
        "\
-Users-me-Code-alpha (4 sessions, 3 with transcript)

2026-09-05 12:00  {S2}  1 prompts, {s2}, updated 2026-09-05 17:00
    Say hello in exactly three words.

2026-09-01 08:00  {S1}  3 prompts, {s1}, updated 2026-09-01 15:00
    /init --force

2026-08-20 01:00  {S4}  4 prompts, updated 2026-08-20 02:30 [history only]
    Fix the flaky test⏎⏎line1⏎line2⏎line3

?  {S5}  0 prompts, {s5}, updated 2026-09-03 04:00
    (no prompt)

",
        s1 = size(&sb, ALPHA, S1),
        s2 = size(&sb, ALPHA, S2),
        s5 = size(&sb, ALPHA, S5),
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}

#[test]
fn list_matches_several_projects_and_honours_the_limit() {
    let sb = sandbox();
    let out = asm(&sb).args(["list", "code", "-n", "2"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("-Users-me-Code-alpha (4 sessions, 3 with transcript)\n"));
    assert!(text.contains("\n… limit of 2 sessions reached, use -n to show more\n"));
    assert!(!text.contains(S4), "third session is past the limit");
    assert!(text.contains("-Users-me-Code-beta (3 sessions, 1 with transcript)"));
    assert!(text.contains("-Users-me-Code-gamma (1 sessions, 0 with transcript)"));
}

#[test]
fn list_shows_anonymous_history_sessions_and_full_prompts() {
    let sb = sandbox();
    let out = asm(&sb).args(["list", "beta", "--full"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("2025-06-01 05:30  (unknown session id)  2 prompts, updated 2025-06-01 05:31 [history only]\n    now add a CLI\n"));
    assert!(text.contains("2025-06-01 00:00  (unknown session id)  2 prompts, updated 2025-06-01 00:30 [history only]\n    design a plagiarism checker\n"));
    let out = asm(&sb).args(["list", "alpha", "--full"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("[history only]\n    Fix the flaky test\n    \n    line1\n    line2\n    line3\n"), "{text}");
}

#[test]
fn list_uses_the_history_alone_when_no_project_dir_exists() {
    let sb = sandbox();
    let out = asm(&sb).args(["list", "gamma"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("-Users-me-Code-gamma (1 sessions, 0 with transcript)\n\n2026-09-12 00:00  66666666-aaaa-4bbb-8ccc-000000000006  1 prompts, updated 2026-09-12 00:00 [history only]\n    hello gamma\n"));
}

#[test]
fn list_errors_when_nothing_matches() {
    let sb = sandbox();
    let out = asm(&sb).args(["list", "nope"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no project matches 'nope'"));
    let dir = sb.tmp.path().join("some-dir");
    std::fs::create_dir(&dir).unwrap();
    let out = asm(&sb).args(["list", dir.to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no sessions recorded for"));
}

#[test]
fn session_looks_up_by_id_prefix_across_transcripts_and_history() {
    let sb = sandbox();
    let out = asm(&sb).args(["session", "4444"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("-Users-me-Code-alpha\n\n2026-08-20 01:00  {S4}  4 prompts, updated 2026-08-20 02:30 [history only]\n    Fix the flaky test\n    \n    line1\n    line2\n    line3\n\n")
    );
    let out = asm(&sb).args(["session", "3333"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("-Users-me-Code-beta\n\n2026-09-10 02:00  33333333"));
    assert!(text.contains("    帮我重构一下解析器，处理嵌套数组\n"));

    let out = asm(&sb).args(["session", "zzz"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no session matches 'zzz'"));
}

#[test]
fn search_prints_matches_grouped_per_transcript_with_a_summary() {
    let sb = sandbox();
    let out = asm(&sb).args(["search", "parser", "-d", "0"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let expected = format!(
        "
-Users-me-Code-alpha {S1}.jsonl  (updated 2026-09-01 15:00, 3 matches)

09-01 08:00 [assistant] Here is the plan: refactor the parser.

09-01 09:30 [user] <system-reminder>⏎remember to be nice⏎</system-reminder>⏎Please refactor the parser to handle nested arrays

09-02 11:00 [assistant] Done. The parser now handles nested arrays; 14 tests pass.

3 matches in 4 sessions scanned
"
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}

#[test]
fn search_renders_tool_calls_on_several_lines_and_highlights_with_ansi() {
    let sb = sandbox();
    let out = asm(&sb).args(["search", "cargo", "-d", "0", "-l", "tool_use:Bash"]).env_remove("NO_COLOR").output().unwrap();
    let raw = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(raw.contains("\x1b[1;31mcargo\x1b[0m"), "match is highlighted: {raw:?}");
    let text = strip_ansi(&raw);
    assert!(text.contains("\n09-01 10:00 [tool/tool_use:Bash]\n# Run the tests\n$ cargo test -p alpha 2>&1 | tail -5\ntimeout: 120000\n"), "{text}");
    assert!(text.ends_with("1 matches in 4 sessions scanned\n"));
}

#[test]
fn search_reports_the_limit_and_the_empty_range() {
    let sb = sandbox();
    let out = asm(&sb).args(["search", "e", "-d", "0", "-n", "1"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("(showing first 1)"), "{text}");
    let out = asm(&sb).args(["search", "parser", "-d", "0.0001"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "no sessions found in the given range\n");
}

#[test]
fn search_via_the_index_lists_sessions_with_hit_counts() {
    let sb = sandbox();
    let out = asm(&sb).args(["search", "--index", "parser", "-r", "user"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains(&format!("-Users-me-Code-alpha {S1}  (updated 2026-09-01 15:00, 1 matches)")), "{text}");
    assert!(text.contains("Please refactor the parser"), "{text}");
    assert!(text.contains("1 matches in 1 sessions"), "{text}");
    let out = asm(&sb).args(["search", "--index", "谢谢"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("1 matches in 1 sessions"));
}

#[test]
fn index_and_stats_report_what_was_scanned() {
    let sb = sandbox();
    let out = asm(&sb).args(["index"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("4 files"), "{text}");
    assert!(text.contains("8 sessions"), "{text}");
    let out = asm(&sb).args(["stats"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sessions: 8"), "{text}");
    assert!(text.contains("projects: 3"), "{text}");
    let out = asm(&sb).args(["index", "--rebuild"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("4 files"));
}
