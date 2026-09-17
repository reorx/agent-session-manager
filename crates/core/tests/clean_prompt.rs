//! Behavior: `clean_prompt` strips harness wrappers exactly like the Python reference script.

use agent_session_core::claude::clean::{clean_prompt, one_line};

#[test]
fn removes_system_reminder_blocks_and_surrounding_whitespace() {
    let text = "<system-reminder>\nremember to be nice\n</system-reminder>\nPlease refactor";
    assert_eq!(clean_prompt(text), "Please refactor");
}

#[test]
fn removes_every_kind_of_wrapper_block() {
    let text = "<ide_opened_file>a.rs</ide_opened_file> <ide_selection>x</ide_selection>real <command-message>m</command-message>";
    assert_eq!(clean_prompt(text), "real");
}

#[test]
fn wrapper_blocks_span_lines() {
    let text = "<system-reminder>\nline1\nline2\n</system-reminder>\n\nhello\n";
    assert_eq!(clean_prompt(text), "hello");
}

#[test]
fn slash_command_is_rendered_as_name_and_args() {
    let text = "<command-message>init is running…</command-message>\n<command-name>/init</command-name>\n<command-args>--force</command-args>";
    assert_eq!(clean_prompt(text), "/init --force");
}

#[test]
fn slash_command_without_args_has_no_trailing_space() {
    assert_eq!(clean_prompt("<command-name>/clear</command-name>"), "/clear");
    assert_eq!(clean_prompt("<command-name>/clear</command-name>\n<command-args></command-args>"), "/clear");
}

#[test]
fn plain_text_is_only_trimmed() {
    assert_eq!(clean_prompt("  hello world \n"), "hello world");
    assert_eq!(clean_prompt(""), "");
}

#[test]
fn unclosed_wrapper_is_left_alone() {
    assert_eq!(clean_prompt("<system-reminder>oops"), "<system-reminder>oops");
}

#[test]
fn one_line_marks_line_breaks_and_cuts_at_240_chars() {
    assert_eq!(one_line("a\nb\n\nc", 240), "a⏎b⏎⏎c");
    let long: String = "é".repeat(300);
    let cut = one_line(&long, 240);
    assert_eq!(cut.chars().count(), 241, "240 chars plus the ellipsis");
    assert!(cut.ends_with('…'));
    assert_eq!(one_line(&"x".repeat(240), 240).chars().count(), 240, "exactly 240 is not cut");
}
