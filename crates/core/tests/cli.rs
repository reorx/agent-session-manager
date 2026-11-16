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
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CLAUDE_CODE_SESSION_ID");
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
fn list_includes_project_directories_without_transcripts_in_name_order() {
    let sb = sandbox();
    let projects = sb.root().join("projects");
    std::fs::create_dir_all(projects.join("-Users-me-Code-empty-z/memory")).unwrap();
    std::fs::create_dir_all(projects.join("-Users-me-Code-empty-a")).unwrap();
    std::fs::write(projects.join("-Users-me-Code-empty-a/sessions-index.json"), "{}").unwrap();
    std::fs::write(projects.join("-Users-me-Code-empty-file"), "").unwrap();
    let out = asm(&sb).args(["list", "Code-empty"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), concat!(
        "-Users-me-Code-empty-a (0 sessions, 0 with transcript)\n\n",
        "-Users-me-Code-empty-z (0 sessions, 0 with transcript)\n\n",
    ));
}

#[test]
fn session_old_looks_up_by_id_prefix_across_transcripts_and_history() {
    let sb = sandbox();
    let out = asm(&sb).args(["session-old", "4444"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("-Users-me-Code-alpha\n\n2026-08-20 01:00  {S4}  4 prompts, updated 2026-08-20 02:30 [history only]\n    Fix the flaky test\n    \n    line1\n    line2\n    line3\n\n")
    );
    let out = asm(&sb).args(["session-old", "3333"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("-Users-me-Code-beta\n\n2026-09-10 02:00  33333333"));
    assert!(text.contains("    帮我重构一下解析器，处理嵌套数组\n"));

    let out = asm(&sb).args(["session-old", "zzz"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no session matches 'zzz'"));
}

#[test]
fn search_old_prints_matches_grouped_per_transcript_with_a_summary() {
    let sb = sandbox();
    let out = asm(&sb).args(["search-old", "parser", "-d", "0"]).output().unwrap();
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
fn search_old_renders_tool_calls_on_several_lines_and_highlights_with_ansi() {
    let sb = sandbox();
    // `asm()` sets NO_COLOR=1: an explicit `--color always` wins over it
    let out = asm(&sb).args(["search-old", "cargo", "-d", "0", "-l", "tool_use:Bash", "--color", "always"]).output().unwrap();
    let raw = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(raw.contains("\x1b[1;31mcargo\x1b[0m"), "match is highlighted: {raw:?}");
    let text = strip_ansi(&raw);
    assert!(text.contains("\n09-01 10:00 [tool/tool_use:Bash]\n# Run the tests\n$ cargo test -p alpha 2>&1 | tail -5\ntimeout: 120000\n"), "{text}");
    assert!(text.ends_with("1 matches in 4 sessions scanned\n"));
}

#[test]
fn output_to_a_pipe_has_no_colors_unless_asked_for() {
    let sb = sandbox();
    let piped = |args: &[&str]| {
        let out = asm(&sb).args(args).env_remove("NO_COLOR").output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        out.stdout.contains(&0x1b)
    };
    for args in [&["list", "alpha"][..], &["search-old", "cargo", "-d", "0"], &["session-old", "1111"], &["cost"]] {
        assert!(!piped(args), "{args:?} printed an escape code into a pipe");
    }
    assert!(piped(&["list", "alpha", "--color", "always"]));
    assert!(piped(&["--color", "always", "list", "alpha"]), "before the command name too");
    assert!(!piped(&["--color", "never", "list", "alpha"]));
    assert!(!piped(&["list", "alpha", "--color", "always", "--no-color"]), "--no-color is --color never");
    assert!(!piped(&["--no-color", "list", "alpha", "--color", "always"]), "and wins over --color wherever it is");
}

#[test]
fn the_old_search_and_session_answer_to_their_new_names_only() {
    let sb = sandbox();
    let out = asm(&sb).args(["search-old", "parser", "-d", "0"]).output().unwrap();
    assert!(out.status.success());
    let out = asm(&sb).args(["session-old", "1111"]).output().unwrap();
    assert!(out.status.success());
    let help = asm(&sb).arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("search-old") && help.contains("session-old"), "{help}");
}

/// What the app does while it writes the index: hold the write lock.
fn hold_write_lock(sb: &Sandbox) -> rusqlite::Connection {
    let app = rusqlite::Connection::open(sb.db_path()).unwrap();
    app.execute_batch("BEGIN IMMEDIATE").unwrap();
    app
}

#[test]
fn a_busy_index_skips_the_refresh_and_answers_from_what_is_indexed() {
    let sb = sandbox();
    asm(&sb).arg("index").assert().success();
    let new_id = "77777777-aaaa-4bbb-8ccc-000000000007";
    std::fs::write(sb.transcript(ALPHA, new_id), format!(
        "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"brand new\"}},\"uuid\":\"u7\",\"timestamp\":\"2026-09-15T00:00:00.000Z\",\"sessionId\":\"{new_id}\",\"cwd\":\"/Users/me/Code/alpha\"}}\n"
    )).unwrap();
    let app = hold_write_lock(&sb);

    let out = asm(&sb).args(["list", "alpha"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stderr), "index busy, results may miss the last few minutes\n");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("-Users-me-Code-alpha (4 sessions, 3 with transcript)\n") && !text.contains(new_id), "answered from the index as it was: {text}");

    let out = asm(&sb).arg("index").output().unwrap();
    assert_eq!(out.status.code(), Some(1), "`asm index` exists to write, so it fails");
    assert!(String::from_utf8_lossy(&out.stderr).contains("locked"), "{}", String::from_utf8_lossy(&out.stderr));

    app.execute_batch("COMMIT").unwrap();
    let out = asm(&sb).args(["list", "alpha"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    assert!(String::from_utf8_lossy(&out.stdout).contains(new_id), "the next refresh catches up");
}

/// What `asm ARGS` answers while another process (the app, or another `asm`) builds the index
/// from nothing: a new index when `rebuild` is false, one the other process empties and rebuilds
/// when it is true. That process holds the write lock for 4 s, longer than a refresh waits for
/// the app before answering from the index as it is.
fn while_another_process_builds(sb: &Sandbox, rebuild: bool, args: &[&str]) -> std::process::Output {
    use agent_session_core::claude::paths::ClaudeDir;
    use agent_session_core::index::{Db, Indexer};
    if rebuild {
        asm(sb).arg("index").assert().success();
    }
    let (db_path, claude) = (sb.db_path(), sb.claude.clone());
    let (holding, held) = std::sync::mpsc::channel();
    let builder = std::thread::spawn(move || {
        let favorites = agent_session_core::config::favorites_dir(&db_path).unwrap().join("claude");
        let indexer = Indexer::new(claude, Some(ClaudeDir::new(favorites)));
        let mut db = Db::open(&db_path).unwrap();
        let mut hold = |p: &agent_session_core::index::Progress| {
            // "indexing" is reported from inside the write transaction
            if p.phase == "indexing" && p.done == 1 {
                holding.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_secs(4));
            }
        };
        match rebuild {
            true => indexer.rebuild(&mut db, &mut hold).unwrap(),
            false => indexer.scan(&mut db, &mut hold).unwrap(),
        };
    });
    held.recv().unwrap();
    let out = asm(sb).args(args).output().unwrap();
    builder.join().unwrap();
    out
}

fn assert_waited_for_the_build(out: &std::process::Output) {
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(
        out.status.success() && stderr.is_empty() && stdout.starts_with("-Users-me-Code-alpha (4 sessions, 3 with transcript)\n"),
        "should answer from the finished build, with no notice that anything is missing; got {:?}, stderr {stderr:?}, stdout {stdout:?}",
        out.status.code()
    );
}

#[test]
fn a_command_on_a_new_index_another_process_is_building_waits_for_the_build() {
    let sb = sandbox();
    assert_waited_for_the_build(&while_another_process_builds(&sb, false, &["list", "alpha"]));
}

#[test]
fn a_command_on_an_index_another_process_is_rebuilding_waits_for_the_rebuild() {
    let sb = sandbox();
    assert_waited_for_the_build(&while_another_process_builds(&sb, true, &["list", "alpha"]));
}

/// `asm index` (to warm up a new index, say) waits for another process's build like a refresh
/// does, and reports the files that process indexed.
#[test]
fn asm_index_on_a_new_index_another_process_is_building_waits_for_the_build() {
    let sb = sandbox();
    let out = while_another_process_builds(&sb, false, &["index"]);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(
        out.status.success() && stdout.starts_with("4 files (0 new, 0 appended, 0 rebuilt, 0 unchanged, 0 missing, 4 indexed meanwhile by another process), "),
        "got {:?}, stderr {stderr:?}, stdout {stdout:?}",
        out.status.code()
    );
}

/// `--no-scan` answers from the index as it is, which an index no scan has gone through cannot
/// do: there it is ignored.
#[test]
fn no_scan_is_ignored_on_an_index_no_scan_has_gone_through() {
    let sb = sandbox();
    let out = asm(&sb).args(["list", "alpha", "--no-scan"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && stdout.starts_with("-Users-me-Code-alpha (4 sessions, 3 with transcript)\n"), "got {:?}, stdout {stdout:?}", out.status.code());

    let new_id = "77777777-aaaa-4bbb-8ccc-000000000007";
    std::fs::write(sb.transcript(ALPHA, new_id), format!(
        "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"brand new\"}},\"uuid\":\"u7\",\"timestamp\":\"2026-09-15T00:00:00.000Z\",\"sessionId\":\"{new_id}\",\"cwd\":\"/Users/me/Code/alpha\"}}\n"
    )).unwrap();
    let out = asm(&sb).args(["list", "alpha", "--no-scan"]).output().unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains(new_id), "once scanned, --no-scan skips the refresh");
}

/// The CLI prints only the outermost message of most errors; for an index it cannot open, that
/// message has to carry the reason.
#[test]
fn an_index_file_that_cannot_be_written_is_reported_as_such_not_as_locked() {
    use std::os::unix::fs::PermissionsExt;
    let sb = sandbox();
    rusqlite::Connection::open(sb.db_path()).unwrap().execute_batch("CREATE TABLE other(x);").unwrap();
    std::fs::set_permissions(sb.db_path(), std::fs::Permissions::from_mode(0o444)).unwrap();
    let out = asm(&sb).args(["list", "alpha"]).output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert_eq!(stderr, format!("open {}: attempt to write a readonly database\n", sb.db_path().display()), "the reason, once");
    assert!(out.stdout.is_empty());
}

#[test]
fn a_refresh_that_fails_for_another_reason_than_the_lock_is_still_an_error() {
    let sb = sandbox();
    asm(&sb).arg("index").assert().success();
    let out = asm(&sb).args(["list", "alpha"]).env("ASM_FAVORITES_DIR", sb.tmp.path().join("other")).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("index --rebuild"), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(out.stdout.is_empty());
}

#[test]
fn an_index_from_another_version_is_not_rebuilt_while_the_app_writes_it() {
    let sb = sandbox();
    asm(&sb).arg("index").assert().success();
    rusqlite::Connection::open(sb.db_path()).unwrap().execute_batch(&format!("PRAGMA user_version = {};", agent_session_core::index::SCHEMA_VERSION - 1)).unwrap();
    let app = hold_write_lock(&sb);
    let out = asm(&sb).args(["list", "alpha"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("needs rebuilding") && stderr.contains("--db"), "{stderr}");
    let sessions: i64 = rusqlite::Connection::open(sb.db_path()).unwrap().query_row("SELECT count(*) FROM sessions", [], |r| r.get(0)).unwrap();
    assert_eq!(sessions, 8, "the index the app writes is still there");
    drop(app);
    asm(&sb).args(["list", "alpha"]).assert().success();
}

#[test]
fn search_old_reports_the_limit_and_the_empty_range() {
    let sb = sandbox();
    let out = asm(&sb).args(["search-old", "e", "-d", "0", "-n", "1"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("(showing first 1)"), "{text}");
    let out = asm(&sb).args(["search-old", "parser", "-d", "0.0001"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "no sessions found in the given range\n");
}

#[test]
fn search_old_via_the_index_lists_sessions_with_hit_counts() {
    let sb = sandbox();
    let out = asm(&sb).args(["search-old", "--index", "parser", "-r", "user"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains(&format!("-Users-me-Code-alpha {S1}  (updated 2026-09-01 15:00, 1 matches)")), "{text}");
    assert!(text.contains("Please refactor the parser"), "{text}");
    assert!(text.contains("1 matches in 1 sessions"), "{text}");
    let out = asm(&sb).args(["search-old", "--index", "谢谢"]).output().unwrap();
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

#[test]
fn cost_lists_a_projects_sessions_in_a_date_range_most_expensive_first() {
    let sb = sandbox();
    let out = asm(&sb).args(["cost", "-p", "alpha", "--since", "2026-09-01", "--until", "2026-09-03"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let expected = concat!(
        "     COST        CC   CTX  UPDATED           SESSION   PROJECT           TITLE\n",
        "    $0.13     $1.23    1%  2026-09-01 15:00  11111111  alpha             Parser refactor\n",
        "        —         —     —  2026-09-03 04:00  55555555  alpha             \n",
        "\n",
        "2 sessions · $0.13 (CC $1.23)\n",
        "tokens: in 62 · out 2k · cache read 89k · cache write 4k\n",
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected, "the end date is included, S2 (09-05) and S4 (08-20) are not");
}

#[test]
fn cost_sorts_by_context_and_puts_sessions_without_data_last() {
    let sb = sandbox();
    let out = asm(&sb).args(["cost", "--sort", "context", "-n", "4"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let ids: Vec<&str> = text.lines().skip(1).take(4).map(|l| l.split_whitespace().nth(5).unwrap_or("")).collect();
    assert_eq!(ids, ["11111111", "22222222", "33333333", "66666666"], "{text}");
    assert!(text.contains("\n8 sessions · $0.13 (CC $1.23) (top 4 shown)\n"), "{text}");
}

#[test]
fn cost_compare_reports_how_far_the_priced_cost_is_from_claude_codes_own() {
    let sb = sandbox();
    let out = asm(&sb).args(["cost", "--compare"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("sessions with a cost recorded by Claude Code: 1\n"), "{text}");
    assert!(text.contains("  >10% lower        1  (100.0%)\n"), "{text}");
    assert!(text.contains("total: $0.13 priced here vs $1.23 recorded (-89.51%)\n"), "{text}");
}

fn catalog_for_cost(sb: &Sandbox) {
    let dir = sb.root().join("cache/model-catalog");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("test-cc.json"), r#"{"fetchedAt":1793664000000,"catalog":{"config":{"models":[{"id":"claude-opus-5","runtime":{"max_input_tokens":100000}}]}}}"#).unwrap();
    std::fs::write(dir.join("broken-cc.json"), "{").unwrap();
}

#[test]
fn cost_uses_the_catalog_window_without_changing_costs() {
    let sb = sandbox();
    catalog_for_cost(&sb);
    let out = asm(&sb).args(["cost", "-p", "alpha", "--since", "2026-09-01", "--until", "2026-09-03"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("    $0.13     $1.23   14%  2026-09-01 15:00  11111111"), "{text}");
}

#[test]
fn cost_windows_reports_files_models_and_changed_sessions_in_the_filtered_range() {
    let sb = sandbox();
    catalog_for_cost(&sb);
    let out = asm(&sb).args(["cost", "--windows", "-p", "alpha", "--since", "2026-09-01", "--until", "2026-09-03", "-n", "1"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("model catalog:") && text.contains("test-cc.json") && text.contains("broken-cc.json") && text.contains("skipped"), "{text}");
    assert!(text.contains("MODEL") && text.contains("CATALOG") && text.contains("BUILT-IN"), "{text}");
    let row = text.lines().find(|l| l.starts_with("claude-opus-5 ")).unwrap();
    assert_eq!(row.split_whitespace().collect::<Vec<_>>(), ["claude-opus-5", "100k", "1M", "1", "1"]);
    assert!(text.contains("sessions with context data: 1 · Ctx changed: 1"), "{text}");
    assert!(text.contains("11111111") && text.contains("1% → 14%"), "{text}");
    assert!(!text.contains("22222222"), "outside the date range");
    let out = asm(&sb).args(["cost", "--windows", "--no-scan", "-n", "0"]).output().unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains("11111111"), "limit applies to changed session details");
}

#[test]
fn cost_windows_and_compare_are_mutually_exclusive() {
    let sb = sandbox();
    let out = asm(&sb).args(["cost", "--windows", "--compare"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot be used with"));
}

#[test]
fn list_orders_sessions_without_a_prompt_by_last_update_then_by_id_like_the_reference() {
    // The reference script and `asm` share this order, so `make parity` never depends on how a
    // directory happens to be listed.
    let sb = sandbox();
    let empty = |sid: &str, mtime: i64| {
        let path = sb.transcript(ALPHA, sid);
        std::fs::write(&path, format!("{{\"type\":\"permission-mode\",\"permissionMode\":\"plan\",\"sessionId\":\"{sid}\"}}\n")).unwrap();
        set_mtime(&path, mtime);
    };
    empty("aaaaaaaa-0000-4000-8000-000000000001", MTIME_S5 + 3_600_000);
    empty("ffffffff-0000-4000-8000-000000000002", MTIME_S5);
    let out = asm(&sb).args(["list", "alpha"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let no_prompt: Vec<&str> = text.lines().filter(|l| l.starts_with("?  ")).map(|l| l.split_whitespace().nth(1).unwrap()).collect();
    assert_eq!(no_prompt, ["aaaaaaaa-0000-4000-8000-000000000001", S5, "ffffffff-0000-4000-8000-000000000002"], "{text}");
}

#[test]
fn favorites_roundtrip_and_last_copy_protection() {
    let sb = sandbox();
    asm(&sb).args(["favorite", "1111"]).assert().success();
    asm(&sb).args(["list", "alpha", "--favorites"]).assert().success().stdout(predicates::str::contains(S1));
    asm(&sb).args(["unfavorite", "1111"]).env("ASM_TRASH_DIR", sb.tmp.path().join("trash")).assert().success();
    asm(&sb).args(["favorite", "1111"]).assert().success();
    std::fs::remove_file(sb.transcript(ALPHA, S1)).unwrap();
    asm(&sb).args(["unfavorite", "1111"]).assert().failure().stderr(predicates::str::contains("--yes"));
    asm(&sb).args(["unfavorite", "1111", "--yes"]).env("ASM_TRASH_DIR", sb.tmp.path().join("trash")).assert().success();
}
#[test]
fn favorites_directory_migrates_and_roots_are_guarded() {
    let sb = sandbox(); asm(&sb).args(["favorite", "1111"]).assert().success();
    let new = sb.tmp.path().join("moved");
    asm(&sb).arg("favorites-dir").arg(&new).assert().success();
    asm(&sb).args(["session-old", "1111"]).assert().success().stdout(predicates::str::contains(new.to_string_lossy().as_ref()));
    asm(&sb).arg("index").env("ASM_FAVORITES_DIR", sb.tmp.path().join("other")).assert().failure().stderr(predicates::str::contains("index --rebuild"));
}

#[test]
fn cost_as_reprices_each_sessions_main_model_and_totals_the_change() {
    let sb = sandbox();
    let out = asm(&sb).args(["cost", "-p", "alpha", "--as", "fable-5-2"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let header = lines.next().unwrap();
    assert!(header.contains("COST") && header.contains("FABLE 5.2") && header.contains("CHANGE") && header.contains("MODEL"), "{text}");
    let s1 = text.lines().find(|l| l.contains("11111111")).unwrap();
    assert!(s1.contains("Opus 5 "), "S1 runs on Opus 5: {s1}");
    assert!(text.contains("main model of each session as Fable 5.2"), "{text}");
    for line in ["input", "cache write 5m", "cache write 1h", "cache read", "output", "other models", "total"] {
        assert!(text.lines().any(|l| l.trim_start().starts_with(line)), "a `{line}` line: {text}");
    }
}

#[test]
fn cost_model_keeps_the_sessions_whose_main_model_it_is() {
    let sb = sandbox();
    let out = asm(&sb).args(["cost", "--model", "haiku-4-5"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("22222222") && !text.contains("11111111"), "S2 runs on Haiku 4.5, S1 on Opus 5: {text}");
    let out = asm(&sb).args(["cost", "--as", "opus-9"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no price for model `opus-9`"));
}

// ---------------------------------------------------------------------------------------------
// The commands for agents: `search`, `session`, `projects` (skills/asm/references/cli.md)

/// stdout, stderr and the exit code of `asm ARGS`, with the environment `asm()` sets.
fn run(cmd: &mut Command) -> (Option<i32>, String, String) {
    let out = cmd.output().unwrap();
    (out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// The elapsed time is the only part of the output that changes from run to run.
fn steady(text: &str) -> String {
    regex::Regex::new(r"(?m) · \d+\.\d\ds$").unwrap().replace_all(text, " · 0.00s").into_owned()
}

fn json_out(cmd: &mut Command) -> serde_json::Value {
    let (code, stdout, stderr) = run(cmd);
    assert_eq!(code, Some(0), "{stderr}");
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {stdout}"))
}

/// A session written into the sandbox, whose working directory is `dir`; its transcript's
/// mtime is `mtime` (unix ms), or now.
fn add_session(sb: &Sandbox, dir: &std::path::Path, id: &str, prompt: &str, mtime: Option<i64>) -> std::path::PathBuf {
    let key = agent_session_core::claude::paths::encode_project_path(&dir.to_string_lossy());
    let path = sb.root().join("projects").join(key).join(format!("{id}.jsonl"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let record = |kind: &str, uuid: &str, message: serde_json::Value| {
        serde_json::json!({"type": kind, "message": message, "uuid": uuid, "timestamp": "2026-09-20T00:00:00.000Z", "sessionId": id, "cwd": dir, "gitBranch": "main"})
            .to_string()
    };
    let user = record("user", "u1", serde_json::json!({"role": "user", "content": prompt}));
    let reply = record("assistant", "a1", serde_json::json!({"role": "assistant", "model": "claude-opus-5", "id": "m1", "content": [{"type": "text", "text": "On it."}]}));
    std::fs::write(&path, format!("{user}\n{reply}\n")).unwrap();
    if let Some(ms) = mtime {
        set_mtime(&path, ms);
    }
    path
}

/// The keys of the JSON example in `### asm <command>` of cli.md, to compare with real output.
fn spec_json_example(command: &str) -> serde_json::Value {
    let spec = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/asm/references/cli.md")).unwrap();
    let section = &spec[spec.find(&format!("\n### asm {command}\n")).unwrap_or_else(|| panic!("no section for {command}"))..];
    let start = section.find("```json\n").expect("a JSON example") + "```json\n".len();
    let end = start + section[start..].find("```").unwrap();
    serde_json::from_str(&section[start..end]).unwrap_or_else(|e| panic!("the {command} example is not JSON: {e}"))
}

/// Where the keys of `spec` and `actual` differ, object by object; arrays compare their first
/// elements, and `free` paths hold data as keys (a tool name per key).
fn shape_diffs(spec: &serde_json::Value, actual: &serde_json::Value, path: &str, free: &[&str], diffs: &mut Vec<String>) {
    use serde_json::Value;
    match (spec, actual) {
        (Value::Object(a), Value::Object(b)) if !free.contains(&path) => {
            let (ka, kb): (std::collections::BTreeSet<&String>, std::collections::BTreeSet<&String>) = (a.keys().collect(), b.keys().collect());
            if ka != kb {
                diffs.push(format!("{path}: spec has {:?}, the output {:?}", ka.difference(&kb).collect::<Vec<_>>(), kb.difference(&ka).collect::<Vec<_>>()));
            }
            for k in ka.intersection(&kb) {
                shape_diffs(&a[*k], &b[*k], &format!("{path}.{k}"), free, diffs);
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if let (Some(x), Some(y)) = (a.first(), b.first()) {
                shape_diffs(x, y, &format!("{path}[]"), free, diffs);
            }
        }
        _ => {}
    }
}

fn assert_same_shape(command: &str, actual: &serde_json::Value, free: &[&str]) {
    let mut diffs = Vec::new();
    shape_diffs(&spec_json_example(command), actual, "", free, &mut diffs);
    assert!(diffs.is_empty(), "cli.md's `asm {command}` JSON example and the output differ:\n{}\n{actual:#}", diffs.join("\n"));
}

#[test]
fn search_lists_each_session_once_with_its_hits_snippets_turns_and_resume_command() {
    let sb = sandbox();
    let (code, stdout, stderr) = run(asm(&sb).args(["search", "parser"]));
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(steady(&stdout), format!("\
1 session · terms [parser] · in prompt,reply,thinking · all time · 0.00s

{S1}
  updated 2026-09-01 15:00 · started 2026-09-01 08:00 · alpha (feat/parser) · 4 turns · $0.13
  title: Parser refactor
  prompt: /init --force
  hits: prompt 1 · reply 2 · thinking 0
  [prompt · turn 2 · 09-01 09:30] …⏎remember to be nice⏎</system-reminder>⏎Please refactor the «parser» to handle nested arrays
  [reply · turn 1 · 09-01 08:00] Here is the plan: refactor the «parser».
  [reply · turn 4 · 09-02 11:00] Done. The «parser» now handles nested arrays; 14 tests pass.
  resume: claude --resume {S1}
"));
}

#[test]
fn search_leaves_out_the_current_session_unless_asked() {
    let sb = sandbox();
    let (_, stdout, _) = run(asm(&sb).args(["search", "parser"]).env("CLAUDE_CODE_SESSION_ID", S1));
    assert_eq!(steady(&stdout), "0 sessions · terms [parser] · in prompt,reply,thinking · all time · 0.00s\ntry: --in all for every kind\n");
    let (_, stdout, _) = run(asm(&sb).args(["search", "parser", "--include-self"]).env("CLAUDE_CODE_SESSION_ID", S1));
    assert!(stdout.starts_with("1 session · ") && stdout.contains(&format!("\n{S1}\n")), "{stdout}");

    let meta = |args: &[&str]| json_out(asm(&sb).args(["search", "parser", "--json"]).args(args).env("CLAUDE_CODE_SESSION_ID", S1))["meta"].clone();
    assert_eq!(meta(&[])["excluded_session"], S1);
    assert_eq!(meta(&[])["total"], 0);
    assert_eq!(meta(&["--include-self"])["excluded_session"], serde_json::Value::Null);
    assert_eq!(meta(&["--include-self"])["total"], 1);
}

#[test]
fn search_needs_every_term_somewhere_in_the_session_or_in_one_item_with_same_message() {
    let sb = sandbox();
    // `parser` is in prompts and replies, `screenshot` in another prompt and in thinking
    let (_, stdout, _) = run(asm(&sb).args(["search", "parser", "screenshot", "--snippets", "2"]));
    assert_eq!(steady(&stdout), format!("\
1 session · terms [parser] [screenshot] · in prompt,reply,thinking · all time · 0.00s

{S1}
  updated 2026-09-01 15:00 · started 2026-09-01 08:00 · alpha (feat/parser) · 4 turns · $0.13
  title: Parser refactor
  prompt: /init --force
  hits: prompt 2 · reply 2 · thinking 1
  [prompt · turn 2 · 09-01 09:30] …⏎remember to be nice⏎</system-reminder>⏎Please refactor the «parser» to handle nested arrays
  [prompt · turn 4 · 09-01 10:00] What is in this «screenshot»?
  resume: claude --resume {S1}
"));
    let (_, stdout, _) = run(asm(&sb).args(["search", "parser", "screenshot", "--same-message"]));
    assert_eq!(
        steady(&stdout),
        "0 sessions · terms [parser] [screenshot] · same message · in prompt,reply,thinking · all time · 0.00s\ntry: --in all for every kind · fewer terms · without --same-message\n"
    );
}

#[test]
fn search_cuts_the_list_at_the_limit_and_says_so() {
    let sb = sandbox();
    let (_, stdout, _) = run(asm(&sb).args(["search", "parser|解析器", "-n", "1", "--snippets", "0"]));
    assert_eq!(steady(&stdout), format!("\
2 sessions (showing 1) · terms [parser|解析器] · in prompt,reply,thinking · all time · 0.00s

{S3}
  updated 2026-09-10 08:00 · started 2026-09-10 02:00 · beta (master) · 2 turns · $0.00
  title: parser-refactor
  prompt: 帮我重构一下解析器，处理嵌套数组
  hits: prompt 1 · reply 2 · thinking 0
  resume: claude --resume {S3}

showing 1 of 2 sessions, use -n for more
"));
    let meta = json_out(asm(&sb).args(["search", "parser|解析器", "-n", "1", "--json"]))["meta"].clone();
    assert_eq!((meta["total"].as_u64(), meta["shown"].as_u64(), meta["truncated"].as_bool()), (Some(2), Some(1), Some(true)));
    let ids = |args: &[&str]| -> Vec<String> {
        let v = json_out(asm(&sb).args(["search", "--json", "-n", "0"]).args(args));
        v["results"].as_array().unwrap().iter().map(|r| r["session_id"].as_str().unwrap().to_string()).collect()
    };
    // S3 is the newer one; S1 has more hits in tool calls
    assert_eq!(ids(&["test|tests|检查", "--in", "all", "-d", "0"]), [S3, S1]);
    assert_eq!(ids(&["test|tests|检查", "--in", "all", "-d", "0", "--sort", "hits"]), [S1, S3]);
}

#[test]
fn search_scans_tool_calls_in_the_last_30_days_unless_given_a_range() {
    let sb = sandbox();
    let (_, stdout, _) = run(asm(&sb).args(["search", "cargo", "--in", "tool"]));
    assert_eq!(
        steady(&stdout),
        "0 sessions · terms [cargo] · in tool-input,tool-output · last 30 days (default) · 0.00s\ntry: --days 0 for all time · --in all for every kind\n",
        "the fixture's sessions are older than 30 days"
    );
    let (_, stdout, _) = run(asm(&sb).args(["search", "cargo", "--in", "tool", "-d", "0"]));
    assert_eq!(steady(&stdout), format!("\
1 session · terms [cargo] · in tool-input,tool-output · all time · 0.00s

{S1}
  updated 2026-09-01 15:00 · started 2026-09-01 08:00 · alpha (feat/parser) · 4 turns · $0.13
  title: Parser refactor
  prompt: /init --force
  hits: tool-input 2 · tool-output 1
  [tool-input Read · turn 1 · 09-01 08:00] file_path: /Users/me/Code/alpha/«Cargo».toml⏎limit: 40
  [tool-input Bash · turn 4 · 09-01 10:00] # Run the tests⏎$ «cargo» test -p alpha 2>&1 | tail -5⏎timeout: 120000
  [tool-output Bash · turn 1 · 09-01 08:00] total 0⏎src⏎«Cargo».toml
  resume: claude --resume {S1}
"));
    // --tool alone searches only that tool's calls
    let (_, stdout, _) = run(asm(&sb).args(["search", "cargo", "--tool", "bash", "-d", "0"]));
    assert!(steady(&stdout).starts_with("1 session · terms [cargo] · in tool-input,tool-output · tools bash · all time · 0.00s\n"), "{stdout}");
    assert!(stdout.contains("hits: tool-input 1 · tool-output 1\n") && !stdout.contains("tool-input Read"), "{stdout}");
    let meta = json_out(asm(&sb).args(["search", "cargo", "--in", "tool", "-d", "0", "--json"]))["meta"].clone();
    assert_eq!(meta["engine"], "scan");
    assert_eq!(meta["scan"]["files"], 4);
}

#[test]
fn search_names_the_range_it_used_in_the_first_line_and_in_json() {
    let sb = sandbox();
    let first = |args: &[&str]| {
        let (code, stdout, stderr) = run(asm(&sb).args(["search", "hello|谢谢|parser", "--snippets", "0"]).args(args));
        assert_eq!(code, Some(0), "{stderr}");
        steady(stdout.lines().next().unwrap())
    };
    assert_eq!(first(&[]), "3 sessions · terms [hello|谢谢|parser] · in prompt,reply,thinking · all time · 0.00s");
    assert_eq!(first(&["-d", "7"]), "0 sessions · terms [hello|谢谢|parser] · in prompt,reply,thinking · last 7 days · 0.00s");
    assert_eq!(first(&["--days", "0.5"]), "0 sessions · terms [hello|谢谢|parser] · in prompt,reply,thinking · last 0.5 days · 0.00s");
    assert_eq!(first(&["-d", "0"]), "3 sessions · terms [hello|谢谢|parser] · in prompt,reply,thinking · all time · 0.00s");
    // S1 was last updated 09-01, S2 runs on 09-05, S3 on 09-10
    assert_eq!(first(&["--since", "2026-09-05"]), "2 sessions · terms [hello|谢谢|parser] · in prompt,reply,thinking · since 2026-09-05 · 0.00s");
    assert_eq!(first(&["--until", "2026-09-05"]), "2 sessions · terms [hello|谢谢|parser] · in prompt,reply,thinking · until 2026-09-05 · 0.00s");
    assert_eq!(
        first(&["--since", "2026-09-05", "--until", "2026-09-05"]),
        "1 session · terms [hello|谢谢|parser] · in prompt,reply,thinking · 2026-09-05 to 2026-09-05 · 0.00s"
    );
    let range = json_out(asm(&sb).args(["search", "hello", "--json", "--since", "2026-09-05", "--until", "2026-09-10"]))["meta"]["range"].clone();
    assert_eq!(
        range,
        serde_json::json!({"label": "2026-09-05 to 2026-09-10", "since": "2026-09-05T00:00:00+00:00", "until": "2026-09-11T00:00:00+00:00", "default": false})
    );
    let range = json_out(asm(&sb).args(["search", "hello", "--json", "--in", "tool"]))["meta"]["range"].clone();
    assert_eq!((range["label"].as_str(), range["until"].is_null(), range["default"].as_bool()), (Some("last 30 days (default)"), true, Some(true)));
}

#[test]
fn search_says_what_could_be_widened_when_nothing_is_found() {
    let sb = sandbox();
    let first_two = |args: &[&str]| {
        let (code, stdout, stderr) = run(asm(&sb).arg("search").args(args));
        assert_eq!(code, Some(0), "{stderr}");
        steady(&stdout)
    };
    assert_eq!(
        first_two(&["nothing here", "parser", "-d", "7", "--in", "prompt", "--same-message", "-p", "alpha", "--model", "opus-5", "--favorites"]),
        "0 sessions · terms [nothing here] [parser] · same message · in prompt · project alpha · model opus-5 · favorites · last 7 days · 0.00s\n\
         try: --days 0 for all time · --in all for every kind · fewer terms · without --same-message · without --project · without --model · without --favorites\n"
    );
    assert_eq!(
        first_two(&["cargo", "--tool", "Bash", "-p", "nope", "-p", "beta", "-d", "0"]),
        "0 sessions · terms [cargo] · in tool-input,tool-output · tools Bash · project nope,beta · all time · 0.00s\n\
         try: --in all for every kind · without --project · without --tool\n"
    );
    assert_eq!(first_two(&["nothing here", "--in", "all", "-d", "0"]), "0 sessions · terms [nothing here] · in prompt,reply,thinking,tool-input,tool-output,other · all time · 0.00s\ntry: other ways of saying each TERM\n");
    let meta = json_out(asm(&sb).args(["search", "nothing here", "-p", "alpha", "--json"]))["meta"].clone();
    assert_eq!(meta["hints"], serde_json::json!(["--in all for every kind", "without --project"]));
}

#[test]
fn search_takes_projects_as_text_or_as_a_directory() {
    let sb = sandbox();
    let first = |args: &[&str]| steady(run(asm(&sb).args(["search", "parser|解析器", "--snippets", "0"]).args(args)).1.lines().next().unwrap());
    assert_eq!(first(&["-p", "BETA"]), "1 session · terms [parser|解析器] · in prompt,reply,thinking · project BETA · all time · 0.00s");
    assert_eq!(first(&["-p", "/Users/me/Code/"]), "2 sessions · terms [parser|解析器] · in prompt,reply,thinking · project /Users/me/Code/ · all time · 0.00s");
    assert_eq!(first(&["-p", "/Users/me/Code/alpha/.."]).split(" · ").next(), Some("2 sessions"));
    assert_eq!(first(&["-p", "/Users/me/Code/al"]).split(" · ").next(), Some("0 sessions"), "a directory is not text");
}

#[test]
fn search_takes_a_term_that_starts_with_a_dash_after_a_double_dash() {
    let sb = sandbox();
    let (code, stdout, stderr) = run(asm(&sb).args(["search", "--snippets", "1", "--", "--force"]));
    assert_eq!(code, Some(0), "{stderr}");
    assert!(steady(&stdout).starts_with("1 session · terms [--force] · "), "{stdout}");
    assert!(stdout.contains("<command-args>«--force»</command-args>"), "{stdout}");
    let (code, stdout, stderr) = run(asm(&sb).args(["search", "--force"]));
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("unexpected argument '--force'") && stdout.is_empty(), "{stderr}");
}

#[test]
fn search_rejects_invalid_arguments_with_exit_2_before_touching_the_index() {
    let sb = sandbox();
    for (args, message) in [
        (&["search", ""][..], "error: term 1 has nothing to match\n"),
        (&["search", "a", " | "], "error: term 2 has nothing to match\n"),
        (&["search", "--regex", "x*"], "error: term 1 can match empty text, so it would match everything\n"),
        (&["search", "x", "--in", "nope"], "error: unknown kind 'nope' in --in (kinds: prompt, reply, thinking, tool, tool-input, tool-output, other, all)\n"),
        (&["search", "x", "--tool", "Bash", "--in", "prompt"], "error: --tool needs a tool kind in --in (tool, tool-input or tool-output)\n"),
        (&["search", "x", "--since", "2026-09-10", "--until", "2026-09-01"], "error: --since 2026-09-10 is later than --until 2026-09-01\n"),
        (&["search", "--regex", r"\w{200}", r"x\w{200}"], "error: the TERMs are too large to match together; use fewer or smaller ones\n"),
    ] {
        let (code, stdout, stderr) = run(asm(&sb).args(args));
        assert_eq!((code, stdout.as_str(), stderr.as_str()), (Some(2), "", message), "{args:?}");
    }
    for args in [&["search", "x", "-d", "3", "--since", "2026-09-01"][..], &["search", "x", "--days", "1", "--until", "2026-09-01"], &["search", "x", "--sort", "best"]] {
        let (code, _, stderr) = run(asm(&sb).args(args));
        assert_eq!(code, Some(2), "{args:?}: {stderr}");
        assert!(stderr.contains("cannot be used with") || stderr.contains("invalid value"), "{args:?}: {stderr}");
    }
    assert!(!sb.db_path().exists(), "invalid arguments never open the index");
}

#[test]
fn search_says_which_regex_terms_it_matched_as_text() {
    let sb = sandbox();
    let (code, stdout, stderr) = run(asm(&sb).args(["search", "--regex", "pars(er|e)", "parse(r", "-d", "0"]));
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "term 2 is not a valid regular expression, matched as literal text\n");
    assert!(steady(&stdout).starts_with("0 sessions · terms [pars(er|e)] [parse(r] · regex · in prompt,reply,thinking · all time · 0.00s\n"), "{stdout}");
    let meta = json_out(asm(&sb).args(["search", "--regex", "pars(er|e)", "parse(r", "-d", "0", "--json"]))["meta"].clone();
    assert_eq!(meta["regex_fallback"], serde_json::json!([1]));
    assert_eq!(meta["terms"], serde_json::json!([["pars(er|e)"], ["parse(r"]]));
    assert_eq!(meta["engine"], "scan");
}

#[test]
fn search_json_carries_the_fields_cli_md_lists() {
    let sb = sandbox();
    let v = json_out(asm(&sb).args(["search", "parser|解析器", "nested|嵌套", "--json", "--snippets", "1"]));
    assert_same_shape("search", &v, &[]);
    let meta = &v["meta"];
    assert_eq!(meta["terms"], serde_json::json!([["parser", "解析器"], ["nested", "嵌套"]]));
    assert_eq!(meta["kinds"], serde_json::json!(["prompt", "reply", "thinking"]));
    assert_eq!((meta["engine"].as_str(), meta["index_refresh"].as_str(), meta["scan"].is_null()), (Some("index"), Some("done"), true));
    let r = &v["results"][0];
    assert_eq!(r["session_id"], S3);
    assert_eq!((r["turns"].as_u64(), r["kind"].as_str(), r["main_model"].as_str()), (Some(2), Some("cli"), Some("claude-opus-5")));
    assert_eq!(r["started_at"], "2026-09-10T02:00:00+00:00");
    assert_eq!(r["resume"], format!("claude --resume {S3}"));
    assert_eq!(r["hits"], serde_json::json!({"total": 3, "prompt": 1, "reply": 2, "thinking": 0, "tool_input": null, "tool_output": null, "other": null}));
    assert_eq!(r["term_hits"], serde_json::json!([3, 2]));
    assert_eq!(
        r["snippets"],
        serde_json::json!([{"kind": "prompt", "tool": null, "turn": 1, "time": "2026-09-10T02:00:00+00:00", "offset": 0, "terms": [0, 1], "text": "帮我重构一下«解析器»，处理«嵌套»数组"}])
    );
    let v = json_out(asm(&sb).args(["search", "cargo", "--in", "tool", "-d", "0", "--json"]));
    assert_same_shape("search", &v, &[]);
    let snippet = &v["results"][0]["snippets"][0];
    assert_eq!((snippet["kind"].as_str(), snippet["tool"].as_str(), snippet["turn"].as_u64()), (Some("tool-input"), Some("Read"), Some(1)));
    assert_eq!(v["results"][0]["hits"]["prompt"], serde_json::Value::Null, "a kind not searched");
}

#[test]
fn a_failing_command_for_agents_answers_json_with_an_error_object() {
    use std::os::unix::fs::PermissionsExt;
    let sb = sandbox();
    rusqlite::Connection::open(sb.db_path()).unwrap().execute_batch("CREATE TABLE other(x);").unwrap();
    std::fs::set_permissions(sb.db_path(), std::fs::Permissions::from_mode(0o444)).unwrap();
    for args in [&["search", "parser", "--json"][..], &["session", "1111", "--json"], &["projects", "--json"]] {
        let (code, stdout, stderr) = run(asm(&sb).args(args));
        let message = format!("open {}: attempt to write a readonly database", sb.db_path().display());
        assert_eq!(code, Some(1), "{args:?}");
        assert_eq!(stderr, format!("{message}\n"), "{args:?}");
        let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(v, serde_json::json!({"error": {"code": "failed", "message": message}}), "{args:?}");
    }
}

#[test]
fn commands_for_agents_print_no_color_codes_into_a_pipe() {
    let sb = sandbox();
    for args in [&["search", "parser"][..], &["search", "cargo", "--in", "tool", "-d", "0"], &["session", "1111"], &["projects"]] {
        let (code, stdout, stderr) = run(asm(&sb).args(args).env_remove("NO_COLOR"));
        assert_eq!(code, Some(0), "{args:?}: {stderr}");
        assert!(!stdout.contains('\x1b'), "{args:?} printed an escape code into a pipe: {stdout:?}");
    }
    let (_, stdout, _) = run(asm(&sb).args(["search", "parser", "--color", "always"]));
    assert!(stdout.contains("«\x1b[1;31mparser\x1b[0m»"), "marks stay, colors are added: {stdout:?}");
}

#[test]
fn commands_for_agents_say_whether_they_refreshed_the_index() {
    let sb = sandbox();
    let refresh = |args: &[&str]| {
        let (code, stdout, stderr) = run(asm(&sb).args(args));
        assert_eq!(code, Some(0), "{args:?}: {stderr}");
        let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        (v["meta"]["index_refresh"].as_str().unwrap().to_string(), stderr)
    };
    for args in [&["search", "parser", "--json"][..], &["session", "1111", "--json"], &["projects", "--json"]] {
        let no_scan: Vec<&str> = args.iter().copied().chain(["--no-scan"]).collect();
        assert_eq!(refresh(args).0, "done", "{args:?}");
        assert_eq!(refresh(&no_scan).0, "skipped", "{args:?}");
        let app = hold_write_lock(&sb);
        assert_eq!(refresh(args), ("busy".to_string(), "index busy, results may miss the last few minutes\n".to_string()), "{args:?}");
        drop(app);
    }
}

#[test]
fn session_shows_a_card_for_a_unique_prefix() {
    let sb = sandbox();
    let (code, stdout, stderr) = run(asm(&sb).args(["session", "1111"]));
    assert_eq!(code, Some(0), "{stderr}");
    let path = sb.transcript(ALPHA, S1);
    let kb = std::fs::metadata(&path).unwrap().len() as f64 / 1024.0;
    assert_eq!(stdout, format!("\
{S1}
  title:        Parser refactor
  first prompt: /init --force
  last prompt:  What is in this screenshot?
  project:      /Users/me/Code/alpha
  cwd:          /Users/me/Code/alpha
  branch:       feat/parser
  kind:         cli
  favorite:     no
  started:      2026-09-01 08:00
  updated:      2026-09-01 15:00
  turns:        4 (3 prompts)
  tools:        Bash 2 · Read 1
  files:        1 changed: src/parse.rs
  cost:         $0.13 (Claude Code recorded $1.23)
  tokens:       in 62 · out 2k · cache read 89k · cache write 4k · hit 95.7%
  context:      14k / 1M (1%) at the end · peak 40k
  models:       claude-opus-5 (main), claude-sonnet-5
  subagents:    1
  transcript:   {} ({kb:.1} KB)
  resume:       claude --resume {S1}
", path.display()));
    // the current session is shown like any other
    let (code, stdout, _) = run(asm(&sb).args(["session", S1]).env("CLAUDE_CODE_SESSION_ID", S1));
    assert!(code == Some(0) && stdout.starts_with(&format!("{S1}\n")), "{stdout}");
}

#[test]
fn session_lists_the_candidates_of_an_ambiguous_prefix_and_fails() {
    let sb = sandbox();
    let other = "11111111-bbbb-4bbb-8ccc-000000000009";
    add_session(&sb, std::path::Path::new("/Users/me/Code/delta"), other, "Fix the login page", Some(1789_862_400_000)); // 2026-09-20T00:00:00Z
    let (code, stdout, stderr) = run(asm(&sb).args(["session", "1111"]));
    assert_eq!((code, stdout.as_str()), (Some(1), ""));
    assert_eq!(stderr, format!("prefix '1111' matches 2 sessions:\n  {other}  2026-09-20 00:00  Fix the login page\n  {S1}  2026-09-01 15:00  Parser refactor\n"));

    let (code, stdout, stderr) = run(asm(&sb).args(["session", "1111", "--json"]));
    assert_eq!((code, stderr.as_str()), (Some(1), "prefix '1111' matches 2 sessions:\n"));
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v, serde_json::json!({"error": {"code": "ambiguous", "message": "prefix '1111' matches 2 sessions:", "candidates": [
        {"session_id": other, "updated_at": "2026-09-20T00:00:00+00:00", "title": "Fix the login page"},
        {"session_id": S1, "updated_at": "2026-09-01T15:00:00+00:00", "title": "Parser refactor"},
    ]}}));

    let (code, stdout, stderr) = run(asm(&sb).args(["session", "11111111-b"]));
    assert!(code == Some(0) && stdout.starts_with(&format!("{other}\n")), "a longer prefix is unique: {stderr}");
}

#[test]
fn session_fails_when_nothing_matches() {
    let sb = sandbox();
    let (code, stdout, stderr) = run(asm(&sb).args(["session", "zzz"]));
    assert_eq!((code, stdout.as_str(), stderr.as_str()), (Some(1), "", "no session matches 'zzz'\n"));
    let (code, stdout, stderr) = run(asm(&sb).args(["session", "zzz", "--json"]));
    assert_eq!((code, stderr.as_str()), (Some(1), "no session matches 'zzz'\n"));
    assert_eq!(serde_json::from_str::<serde_json::Value>(&stdout).unwrap(), serde_json::json!({"error": {"code": "not_found", "message": "no session matches 'zzz'"}}));
}

#[test]
fn session_of_a_history_only_session_has_nothing_to_read_or_resume() {
    let sb = sandbox();
    let (_, stdout, _) = run(asm(&sb).args(["session", "4444"]));
    assert!(stdout.contains("\n  kind:         history\n"), "{stdout}");
    assert!(stdout.contains("\n  turns:        0 (4 prompts)\n"), "{stdout}");
    assert!(stdout.ends_with("\n  transcript:   none (history only)\n  resume:       none (history only)\n"), "{stdout}");
    let s = json_out(asm(&sb).args(["session", "4444", "--json"]))["session"].clone();
    assert_eq!((s["transcript"].is_null(), s["transcript_bytes"].is_null(), s["resume"].is_null(), s["kind"].as_str()), (true, true, true, Some("history")));
}

#[test]
fn a_session_whose_project_folder_exists_resumes_from_it() {
    let sb = sandbox();
    let dir = sb.tmp.path().join("My Code").join("O'Brien");
    std::fs::create_dir_all(&dir).unwrap();
    let id = "77777777-aaaa-4bbb-8ccc-000000000007";
    add_session(&sb, &dir, id, "Fix the login page", None);
    let resume = format!("cd '{}' && claude --resume {id}", dir.display().to_string().replace('\'', r"'\''"));
    let (_, stdout, _) = run(asm(&sb).args(["session", id]));
    assert!(stdout.ends_with(&format!("\n  resume:       {resume}\n")), "{stdout}");
    let (_, stdout, _) = run(asm(&sb).args(["search", "login"]));
    assert!(stdout.ends_with(&format!("\n  resume: {resume}\n")), "{stdout}");
    assert_eq!(json_out(asm(&sb).args(["session", id, "--json"]))["session"]["resume"], resume);
}

#[test]
fn a_favorite_whose_original_is_gone_shows_its_copy_and_cannot_be_resumed() {
    let sb = sandbox();
    asm(&sb).args(["favorite", "2222"]).assert().success();
    std::fs::remove_file(sb.transcript(ALPHA, S2)).unwrap();
    let (_, stdout, _) = run(asm(&sb).args(["session", "2222"]));
    assert!(stdout.contains("\n  favorite:     yes\n"), "{stdout}");
    let transcript = stdout.lines().find(|l| l.starts_with("  transcript:")).unwrap();
    assert!(transcript.contains("/favorites/claude/projects/-Users-me-Code-alpha/") && transcript.ends_with(" KB) (favorite copy)"), "{transcript}");
    assert!(stdout.ends_with("\n  resume:       none (only a favorite copy is left)\n"), "{stdout}");
    let s = json_out(asm(&sb).args(["session", "2222", "--json"]))["session"].clone();
    assert_eq!((s["favorite_copy"].as_bool(), s["resume"].is_null()), (Some(true), true));
    let (_, stdout, _) = run(asm(&sb).args(["search", "hello"]));
    assert!(stdout.ends_with("\n  resume: none (only a favorite copy is left)\n"), "{stdout}");
}

#[test]
fn session_json_carries_the_fields_cli_md_lists() {
    let sb = sandbox();
    let v = json_out(asm(&sb).args(["session", "1111", "--json"]));
    assert_same_shape("session", &v, &[".session.tools"]);
    let s = &v["session"];
    assert_eq!(v["meta"]["query"], "1111");
    assert_eq!((s["turns"].as_u64(), s["prompts"].as_u64(), s["subagents"].as_u64()), (Some(4), Some(3), Some(1)));
    assert_eq!(s["tools"], serde_json::json!({"Bash": 2, "Read": 1}));
    assert_eq!(s["files"], serde_json::json!(["src/parse.rs"]));
    assert_eq!(s["models"], serde_json::json!(["claude-opus-5", "claude-sonnet-5"]));
    assert_eq!((s["main_model"].as_str(), s["recorded_cost_usd"].as_f64()), (Some("claude-opus-5"), Some(1.2345)));
    assert_eq!(s["context"]["window"], 1_000_000);
    let hit = s["tokens"]["hit_rate"].as_f64().unwrap();
    assert!((0.957..0.958).contains(&hit), "{hit}");
    assert_eq!(s["transcript_bytes"], std::fs::metadata(sb.transcript(ALPHA, S1)).unwrap().len());
}

#[test]
fn projects_lists_projects_most_recently_active_first() {
    let sb = sandbox();
    let (code, stdout, stderr) = run(asm(&sb).arg("projects"));
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(steady(&stdout), "\
3 projects · all time · 0.00s
2026-09-12 00:00    1 session   /Users/me/Code/gamma
2026-09-10 08:00    3 sessions  /Users/me/Code/beta
2026-09-05 17:00    4 sessions  /Users/me/Code/alpha
");
    let (_, stdout, _) = run(asm(&sb).args(["projects", "-n", "1"]));
    assert_eq!(steady(&stdout), "3 projects (showing 1) · all time · 0.00s\n2026-09-12 00:00    1 session   /Users/me/Code/gamma\nshowing 1 of 3 projects, use -n for more\n");
}

#[test]
fn projects_in_a_range_count_only_the_sessions_active_in_it() {
    let sb = sandbox();
    let (_, stdout, _) = run(asm(&sb).args(["projects", "--since", "2026-09-04"]));
    assert_eq!(steady(&stdout), "\
3 projects · since 2026-09-04 · 0.00s
2026-09-12 00:00    1 session   /Users/me/Code/gamma
2026-09-10 08:00    1 session   /Users/me/Code/beta
2026-09-05 17:00    1 session   /Users/me/Code/alpha
");
    let (_, stdout, _) = run(asm(&sb).args(["projects", "--days", "30"]));
    assert_eq!(steady(&stdout), "0 projects · last 30 days · 0.00s\n");
    let id = "77777777-aaaa-4bbb-8ccc-000000000007";
    add_session(&sb, std::path::Path::new("/Users/me/Code/delta"), id, "Fix the login page", None);
    let (_, stdout, _) = run(asm(&sb).args(["projects", "--days", "30"]));
    assert!(steady(&stdout).starts_with("1 project · last 30 days · 0.00s\n") && stdout.ends_with("    1 session   /Users/me/Code/delta\n"), "{stdout}");

    // the current session neither counts nor keeps its project on the list
    let (_, stdout, _) = run(asm(&sb).args(["projects", "--days", "30"]).env("CLAUDE_CODE_SESSION_ID", id));
    assert_eq!(steady(&stdout), "0 projects · last 30 days · 0.00s\n");
    let (_, stdout, _) = run(asm(&sb).args(["projects", "--days", "30", "--include-self"]).env("CLAUDE_CODE_SESSION_ID", id));
    assert!(stdout.ends_with("/Users/me/Code/delta\n"), "{stdout}");
    let meta = json_out(asm(&sb).args(["projects", "--days", "30", "--json"]).env("CLAUDE_CODE_SESSION_ID", id))["meta"].clone();
    assert_eq!((meta["excluded_session"].as_str(), meta["total"].as_u64()), (Some(id), Some(0)));
}

#[test]
fn projects_json_carries_the_fields_cli_md_lists() {
    let sb = sandbox();
    let v = json_out(asm(&sb).args(["projects", "--json", "--since", "2026-09-04", "-n", "1"]));
    assert_same_shape("projects", &v, &[]);
    assert_eq!(
        v["results"],
        serde_json::json!([{"project_path": "/Users/me/Code/gamma", "project": "gamma", "project_key": GAMMA, "sessions": 1, "updated_at": "2026-09-12T00:00:00+00:00"}])
    );
    let meta = &v["meta"];
    assert_eq!((meta["total"].as_u64(), meta["shown"].as_u64(), meta["truncated"].as_bool()), (Some(3), Some(1), Some(true)));
    assert_eq!(meta["range"]["since"], "2026-09-04T00:00:00+00:00");
}

// ---------------------------------------------------------------------------------------------
// Rework R1 of the commands for agents

/// A transcript of `records` under `projects/<key>/`; its mtime is `mtime` (unix ms), or now.
fn write_transcript(sb: &Sandbox, key: &str, id: &str, records: &[serde_json::Value], mtime: Option<i64>) -> std::path::PathBuf {
    let path = sb.root().join("projects").join(key).join(format!("{id}.jsonl"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let lines: Vec<String> = records.iter().map(|r| r.to_string()).collect();
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    if let Some(ms) = mtime {
        set_mtime(&path, ms);
    }
    path
}

/// A record of session `id`; `ts` and `cwd` are left out when `None`.
fn record(kind: &str, id: &str, uuid: &str, message: serde_json::Value, ts: Option<&str>, cwd: Option<&str>) -> serde_json::Value {
    let mut r = serde_json::json!({"type": kind, "message": message, "uuid": uuid, "sessionId": id, "gitBranch": "main"});
    if let Some(ts) = ts {
        r["timestamp"] = ts.into();
    }
    if let Some(cwd) = cwd {
        r["cwd"] = cwd.into();
    }
    r
}

fn user_text(text: &str) -> serde_json::Value {
    serde_json::json!({"role": "user", "content": text})
}

fn reply_text(msg_id: &str, text: &str) -> serde_json::Value {
    serde_json::json!({"role": "assistant", "model": "claude-opus-5", "id": msg_id, "content": [{"type": "text", "text": text}]})
}

const DELTA: &str = "/Users/me/Code/delta";
const MTIME_DELTA: i64 = 1789_862_400_000; // 2026-09-20T00:00:00Z

/// `asm()`'s environment on a plain command, for a command that needs its own stdout or shell.
fn with_asm_env(mut cmd: std::process::Command, sb: &Sandbox) -> std::process::Command {
    cmd.env("ASM_CLAUDE_DIR", sb.root())
        .env("ASM_DB", sb.db_path())
        .env("TZ", "UTC")
        .env("NO_COLOR", "1")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CLAUDE_CODE_SESSION_ID");
    cmd
}

#[test]
fn session_shows_the_last_prompt_a_person_wrote_not_a_task_notification() {
    let sb = sandbox();
    let id = "77777777-aaaa-4bbb-8ccc-000000000007";
    let notification = "<task-notification>\n<task-id>b9s8pilxu</task-id>\n<status>completed</status>\n<summary>Background command \"cargo test\" completed</summary>\n</task-notification>";
    let ts = |s: &str| Some(format!("2026-09-20T00:0{s}.000Z"));
    let records = [
        record("user", id, "u1", user_text("Fix the login page"), ts("0:00").as_deref(), Some(DELTA)),
        record("assistant", id, "a1", reply_text("m1", "On it."), ts("0:01").as_deref(), Some(DELTA)),
        record("user", id, "u2", user_text("<system-reminder>\nbe nice\n</system-reminder>\n好的，提交吧"), ts("1:00").as_deref(), Some(DELTA)),
        record("assistant", id, "a2", reply_text("m2", "Committed, the tests run in the background."), ts("1:01").as_deref(), Some(DELTA)),
        {
            let mut r = record("user", id, "u3", user_text(notification), ts("2:00").as_deref(), Some(DELTA));
            r["origin"] = serde_json::json!({"kind": "task-notification"});
            r
        },
        record("assistant", id, "a3", reply_text("m3", "The tests passed."), ts("2:01").as_deref(), Some(DELTA)),
    ];
    write_transcript(&sb, "-Users-me-Code-delta", id, &records, None);
    let (code, stdout, stderr) = run(asm(&sb).args(["session", id]));
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("\n  first prompt: Fix the login page\n  last prompt:  好的，提交吧\n"), "{stdout}");
    assert!(stdout.contains("\n  turns:        2 (3 prompts)\n"), "the notification is no turn either: {stdout}");
    let s = json_out(asm(&sb).args(["session", id, "--json"]))["session"].clone();
    assert_eq!((s["first_prompt"].as_str(), s["last_prompt"].as_str()), (Some("Fix the login page"), Some("好的，提交吧")));

    // a session Claude Code kept only in history.jsonl has its last prompt from there
    let (_, stdout, _) = run(asm(&sb).args(["session", "4444"]));
    assert!(stdout.contains("\n  last prompt:  [Pasted text #3 +9 lines]\n"), "{stdout}");
}

#[test]
fn search_for_a_project_that_does_not_exist_suggests_only_dropping_it() {
    let sb = sandbox();
    let (code, stdout, stderr) = run(asm(&sb).args(["search", "parser", "nested", "-p", "nope", "--in", "prompt", "-d", "7"]));
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(steady(&stdout), "0 sessions · terms [parser] [nested] · in prompt · project nope · last 7 days · 0.00s\ntry: without --project\n");
    let meta = json_out(asm(&sb).args(["search", "nothing here", "-p", "nope", "-p", "nada", "--json"]))["meta"].clone();
    assert_eq!(meta["hints"], serde_json::json!(["without --project"]));
    // a project that exists, with nothing in range: every hint that applies
    let (_, stdout, _) = run(asm(&sb).args(["search", "parser", "-p", "alpha", "-d", "7"]));
    assert_eq!(
        steady(&stdout),
        "0 sessions · terms [parser] · in prompt,reply,thinking · project alpha · last 7 days · 0.00s\ntry: --days 0 for all time · --in all for every kind · without --project\n"
    );
}

#[test]
fn search_titles_a_session_by_its_first_prompt_or_no_prompt_and_labels_snippets_without_turn_or_time() {
    let sb = sandbox();
    let id = "77777777-aaaa-4bbb-8ccc-000000000007";
    add_session(&sb, std::path::Path::new(DELTA), id, "Fix the login page\nand the logout button", Some(MTIME_DELTA));
    let (code, stdout, stderr) = run(asm(&sb).args(["search", "logout"]));
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(steady(&stdout), format!("\
1 session · terms [logout] · in prompt,reply,thinking · all time · 0.00s

{id}
  updated 2026-09-20 00:00 · started 2026-09-20 00:00 · delta (main) · 1 turn
  title: Fix the login page
  hits: prompt 1 · reply 0 · thinking 0
  [prompt · turn 1 · 09-20 00:00] Fix the login page⏎and the «logout» button
  resume: claude --resume {id}
"));

    // no prompt with text: a pasted image; a reply before the first prompt has no turn, and no
    // time without a timestamp (nor a start, as the first record has none)
    let id = "88888888-aaaa-4bbb-8ccc-000000000008";
    let image = serde_json::json!({"role": "user", "content": [{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "iVBORw0KGgo="}}]});
    let records = [
        record("assistant", id, "a0", reply_text("m0", "Warming up the quokka cache."), None, Some(DELTA)),
        record("user", id, "u1", image, Some("2026-09-20T00:00:00.000Z"), Some(DELTA)),
        record("assistant", id, "a1", reply_text("m1", "The quokka is smiling."), Some("2026-09-20T00:00:05.000Z"), Some(DELTA)),
    ];
    write_transcript(&sb, "-Users-me-Code-delta", id, &records, Some(MTIME_DELTA));
    let (code, stdout, stderr) = run(asm(&sb).args(["search", "quokka"]));
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(steady(&stdout), format!("\
1 session · terms [quokka] · in prompt,reply,thinking · all time · 0.00s

{id}
  updated 2026-09-20 00:00 · delta (main) · 1 turn
  title: (no prompt)
  hits: prompt 0 · reply 2 · thinking 0
  [reply] Warming up the «quokka» cache.
  [reply · turn 1 · 09-20 00:00] The «quokka» is smiling.
  resume: claude --resume {id}
"));
    let r = json_out(asm(&sb).args(["search", "quokka", "--json"]))["results"][0].clone();
    assert_eq!((r["title"].is_null(), r["first_prompt"].is_null()), (true, true));
    assert_eq!((r["snippets"][0]["turn"].is_null(), r["snippets"][0]["time"].is_null()), (true, true));
}

#[test]
fn session_card_names_the_first_tools_and_files_and_dashes_what_is_unknown() {
    let sb = sandbox();
    let id = "77777777-aaaa-4bbb-8ccc-000000000007";
    let dir = "/Users/me/Code/eps";
    let tool = |n: usize, name: &str| serde_json::json!({"type": "tool_use", "id": format!("t{n}"), "name": name, "input": {}});
    let tools: Vec<serde_json::Value> =
        ["Bash", "Bash", "Read", "Edit", "Write", "Grep", "Glob", "Task", "TodoWrite", "WebFetch"].iter().enumerate().map(|(n, name)| tool(n, name)).collect();
    let mut records = vec![
        record("user", id, "u1", user_text("Tidy up the eps crate"), None, Some(dir)),
        record("assistant", id, "a1", serde_json::json!({"role": "assistant", "model": "claude-opus-5", "id": "m1", "content": tools}), None, Some(dir)),
    ];
    for n in 1..=11 {
        records.push(serde_json::json!({"type": "file-history-delta", "trackingPath": format!("{dir}/src/f{n:02}.rs"), "backup": {"backupFileName": null, "version": 1}}));
    }
    write_transcript(&sb, "-Users-me-Code-eps", id, &records, Some(MTIME_DELTA));
    let (code, stdout, stderr) = run(asm(&sb).args(["session", id]));
    assert_eq!(code, Some(0), "{stderr}");
    let row = |label: &str| stdout.lines().find(|l| l.starts_with(&format!("  {label}:"))).unwrap_or_else(|| panic!("no {label} row: {stdout}")).to_string();
    assert_eq!(row("tools"), "  tools:        Bash 2 · Edit 1 · Glob 1 · Grep 1 · Read 1 · Task 1 · TodoWrite 1 · WebFetch 1 (+1 more)");
    assert_eq!(
        row("files"),
        "  files:        11 changed: src/f01.rs, src/f02.rs, src/f03.rs, src/f04.rs, src/f05.rs, src/f06.rs, src/f07.rs, src/f08.rs, src/f09.rs, src/f10.rs (+1 more)"
    );
    assert_eq!(row("started"), "  started:      —");
    assert_eq!(row("cost"), "  cost:         —");
    let s = json_out(asm(&sb).args(["session", id, "--json"]))["session"].clone();
    assert_eq!((s["files"].as_array().map(Vec::len), s["tools"].as_object().map(|t| t.len())), (Some(11), Some(9)), "JSON is not cut");
}

#[test]
fn search_json_meta_scan_has_the_keys_cli_md_names() {
    let sb = sandbox();
    let spec = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/asm/references/cli.md")).unwrap();
    let row = spec.lines().find(|l| l.starts_with("| `meta.scan` |")).expect("a meta.scan row");
    let read = &row[row.find("what it read:").expect("the keys after 'what it read:'")..row.find("; `null`").unwrap()];
    let named: std::collections::BTreeSet<String> = regex::Regex::new(r"`([a-z_]+)`").unwrap().captures_iter(read).map(|c| c[1].to_string()).collect();
    assert_eq!(named.len(), 4, "{read}");
    let scan = json_out(asm(&sb).args(["search", "cargo", "--in", "tool", "-d", "0", "--json"]))["meta"]["scan"].clone();
    let keys: std::collections::BTreeSet<String> = scan.as_object().expect("scan is an object with the scan engine").keys().cloned().collect();
    assert_eq!(keys, named);
    assert!(scan.as_object().unwrap().values().all(|v| v.is_u64()), "{scan}");
    assert_eq!((scan["files"].as_u64(), scan["skipped_files"].as_u64()), (Some(4), Some(3)), "{scan}");
    assert!(scan["bytes"].as_u64() > scan["skipped_bytes"].as_u64(), "{scan}");
}

#[test]
fn search_does_not_need_the_current_directory_unless_a_project_is_relative() {
    let sb = sandbox();
    let gone = sb.tmp.path().join("gone");
    std::fs::create_dir(&gone).unwrap();
    let asm_in_gone_dir = |args: &str| {
        let script = format!("cd '{d}' && rmdir '{d}' && exec '{asm}' {args}", d = gone.display(), asm = assert_cmd::cargo::cargo_bin("asm").display());
        let mut sh = std::process::Command::new("/bin/sh");
        sh.args(["-c", &script]);
        let out = with_asm_env(sh, &sb).output().unwrap();
        std::fs::create_dir(&gone).unwrap();
        (out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let (code, stdout, stderr) = asm_in_gone_dir("search parser --snippets 0");
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    assert!(steady(&stdout).starts_with("1 session · terms [parser] · "), "{stdout}");
    for p in ["/Users/me/Code/alpha", "~/Code", "alpha"] {
        let (code, _, stderr) = asm_in_gone_dir(&format!("search parser -p '{p}'"));
        assert_eq!((code, stderr.as_str()), (Some(0), ""), "{p}");
    }
    let (code, stdout, stderr) = asm_in_gone_dir("search parser -p ./x");
    assert_eq!((code, stdout.as_str()), (Some(1), ""));
    assert_eq!(stderr, "--project ./x is relative, and the current directory cannot be read: No such file or directory (os error 2)\n");
}

#[test]
fn session_with_an_empty_id_is_an_invalid_argument() {
    let sb = sandbox();
    for args in [&["session", ""][..], &["session", "  "], &["session", "", "--json"]] {
        let (code, stdout, stderr) = run(asm(&sb).args(args));
        assert_eq!((code, stdout.as_str(), stderr.as_str()), (Some(2), "", "error: ID is empty\n"), "{args:?}");
    }
    assert!(!sb.db_path().exists(), "invalid arguments never open the index");
}

#[test]
fn a_project_whose_path_is_unknown_is_listed_by_a_name_that_works_as_project() {
    let sb = sandbox();
    let (key, id) = ("-Users-me-Code-lost", "99999999-aaaa-4bbb-8ccc-000000000009");
    let records = [
        record("user", id, "u1", user_text("Where did the zebra go?"), Some("2026-09-20T00:00:00.000Z"), None),
        record("assistant", id, "a1", reply_text("m1", "It went home."), Some("2026-09-20T00:00:01.000Z"), None),
    ];
    write_transcript(&sb, key, id, &records, Some(MTIME_DELTA));
    let (_, stdout, _) = run(asm(&sb).arg("projects"));
    let line = stdout.lines().nth(1).unwrap().to_string();
    assert_eq!(line, format!("2026-09-20 00:00    1 session   {key}"));
    let printed = line.rsplit("  ").next().unwrap();
    let project = json_out(asm(&sb).args(["projects", "--json"]))["results"][0].clone();
    assert_eq!((project["project_path"].is_null(), project["project"].as_str(), project["project_key"].as_str()), (true, Some(key), Some(key)));
    let joined = format!("--project={printed}");
    for args in [&["-p", printed][..], &["--project", printed], &[joined.as_str()]] {
        let (code, stdout, stderr) = run(asm(&sb).args(["search", "zebra", "--snippets", "0"]).args(args));
        assert_eq!(code, Some(0), "{args:?}: {stderr}");
        assert!(steady(&stdout).starts_with(&format!("1 session · terms [zebra] · in prompt,reply,thinking · project {key} · all time · 0.00s\n\n{id}\n")), "{args:?}: {stdout}");
    }
    // the next word after -p is its value; the option after that value is still an option
    let v = json_out(asm(&sb).args(["search", "zebra", "-p", printed, "--json"]));
    assert_eq!((v["meta"]["projects"].clone(), v["meta"]["total"].as_u64()), (serde_json::json!([key]), Some(1)));
}

#[test]
fn without_asm_db_the_cli_keeps_an_index_of_its_own_next_to_the_apps() {
    let sb = sandbox();
    let home = sb.tmp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let data = home.join("Library/Application Support/dev.reorx.AgentSessionManager");
    let (code, stdout, stderr) = run(asm(&sb).arg("projects").env_remove("ASM_DB").env("HOME", &home));
    assert_eq!(code, Some(0), "{stderr}");
    assert!(steady(&stdout).starts_with("3 projects · all time · "), "{stdout}");
    assert!(data.join("index-cli.sqlite").is_file(), "the CLI's index");
    assert!(!data.join("index.sqlite").exists(), "the app's index is left alone");
    // the favorites (and their configuration) stay the app's: next to both indexes
    let (_, stdout, _) = run(asm(&sb).arg("favorites-dir").env_remove("ASM_DB").env("HOME", &home));
    assert_eq!(stdout, format!("{}\n", data.join("favorites").display()));
    // --db still chooses (as ASM_DB does in every other test)
    let other = sb.tmp.path().join("other.sqlite");
    let (code, _, stderr) = run(asm(&sb).args(["projects", "--db"]).arg(&other).env_remove("ASM_DB").env("HOME", &home));
    assert_eq!(code, Some(0), "{stderr}");
    assert!(other.is_file());
}

#[test]
fn output_cut_short_by_its_reader_is_not_an_error() {
    let sb = sandbox();
    // enough output to fill the 8 KB write buffer of each command below
    let ids: Vec<String> = (0..200).map(|i| format!("99999999-aaaa-4bbb-8ccc-{i:012}")).collect();
    for (i, id) in ids.iter().enumerate() {
        let dir = format!("/Users/me/Code/p{i:03}");
        add_session(&sb, std::path::Path::new(&dir), id, &format!("Fix the login page {}", "and the rest ".repeat(80)), Some(MTIME_DELTA));
    }
    let mut failed = Vec::new();
    for (bin, args) in [
        ("asm", &["search", "login", "-n", "0"][..]),
        ("asm", &["search", "login", "-n", "0", "--json"]),
        ("asm", &["search", "login", "--in", "all", "-d", "0", "-n", "0"]),
        ("asm", &["projects", "-n", "0"]),
        ("asm", &["projects", "-n", "0", "--json"]),
        ("asm", &["transcript", &ids[0]]),
        ("asm", &["search-old", "login", "-d", "0"]),
        ("transcript-reader", &[&ids[0]]),
    ] {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let mut cmd = with_asm_env(std::process::Command::new(assert_cmd::cargo::cargo_bin(bin)), &sb);
        let out = cmd.args(args).stdout(writer).stderr(std::process::Stdio::piped()).output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        if (out.status.code(), stderr.as_ref()) != (Some(0), "") {
            failed.push(format!("{bin} {args:?}: exit {:?}, {stderr:?}", out.status.code()));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
