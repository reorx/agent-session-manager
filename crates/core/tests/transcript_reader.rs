//! Behavior contract for compact session handoffs.
mod common;
use agent_session_core::transcript::{read_transcript, resolve_transcript};
use assert_cmd::Command;
use common::*;

#[test]
fn an_interruption_marker_is_not_exported_as_user_input() {
    let path = fixture_root().parent().unwrap().join("reader.jsonl");
    let export = read_transcript(&path).unwrap();
    assert!(!export.text.contains("[Request interrupted by user"));
    assert_eq!(export.text.matches("<user>").count(), 3);

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("interrupted.jsonl");
    for marker in [
        serde_json::json!({"type":"user", "interruptedMessageId":"api-1", "message":{"content":"marker without standard text"}}),
        serde_json::json!({"type":"user", "message":{"content":[{"type":"text", "text":"  [Request interrupted by user for tool use]"}]}}),
    ] {
        let rows = [
            serde_json::json!({"type":"user", "message":{"content":"question"}}),
            serde_json::json!({"type":"assistant", "message":{"content":"answer", "stop_reason":"end_turn"}}),
            marker,
        ];
        std::fs::write(&path, rows.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let export = read_transcript(&path).unwrap();
        assert_eq!(export.text.matches("<user>").count(), 1);
        assert!(export.text.contains("<agent>\nanswer\n</agent>"));
        assert!(!export.text.contains("marker without standard text"));
        assert!(!export.text.contains("[Request interrupted by user"));
    }
}

#[test]
fn exports_human_inputs_final_answers_and_tool_counts_without_intermediate_noise() {
    let path = fixture_root().parent().unwrap().join("reader.jsonl");
    let export = read_transcript(&path).unwrap();
    assert_eq!(export.text, r#"<transcript session_id="reader">

<user>
修复问题
</user>

<turn-summary>
Called Bash 2 times, Read 1 time.
</turn-summary>
<agent>
完成。

```rust
let x = 1;
```
</agent>

<user>
继续
</user>

<turn-summary>
Called Edit 1 time.
</turn-summary>
<agent status="missing">
[No final response recorded]
</agent>

<user>
[Image omitted]
</user>

<agent>
这是一张图片。
</agent>

</transcript>
"#);
    assert_eq!(export.skipped_lines, 1);
}

#[test]
fn resolves_exact_ids_unique_prefixes_and_paths_without_creating_an_index() {
    let sb = sandbox();
    assert_eq!(resolve_transcript(&sb.claude, None, S1).unwrap(), sb.transcript(ALPHA, S1));
    assert_eq!(resolve_transcript(&sb.claude, None, "1111").unwrap(), sb.transcript(ALPHA, S1));
    let path = sb.transcript(ALPHA, S2);
    assert_eq!(resolve_transcript(&sb.claude, None, path.to_str().unwrap()).unwrap(), path);
    std::fs::copy(sb.transcript(ALPHA, S1), sb.transcript(ALPHA, "1111-other")).unwrap();
    assert!(resolve_transcript(&sb.claude, None, "1111").unwrap_err().to_string().contains("ambiguous"));
    assert!(resolve_transcript(&sb.claude, None, S4).unwrap_err().to_string().contains("no transcript"));
    assert!(!sb.db_path().exists());
}

#[test]
fn both_cli_entrypoints_export_identical_text_and_refuse_overwriting_files() {
    let sb = sandbox();
    let standalone = Command::cargo_bin("transcript-reader").unwrap()
        .env("ASM_CLAUDE_DIR", sb.root()).arg(S2).output().unwrap();
    assert!(standalone.status.success(), "{}", String::from_utf8_lossy(&standalone.stderr));
    let integrated = Command::cargo_bin("asm").unwrap()
        .env("ASM_CLAUDE_DIR", sb.root()).args(["transcript", S2]).output().unwrap();
    assert!(integrated.status.success());
    assert_eq!(standalone.stdout, integrated.stdout);
    let output = sb.tmp.path().join("session.txt");
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
    let export = read_transcript(&path).unwrap();
    assert!(export.text.contains("hello"));
    assert_eq!(export.skipped_lines, 0);
    append(&path, "\n{\"type\":");
    assert_eq!(read_transcript(&path).unwrap().skipped_lines, 1);
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
    let export = read_transcript(&path).unwrap();
    assert!(export.text.contains("<agent>\nFirst final\n</agent>\n\n<turn-summary>\nCalled Read 1 time.\n</turn-summary>\n<agent status=\"missing\">\n"));
    assert!(export.text.contains("[No final response recorded]"));
    assert!(!export.text.contains("Unfinished text"));
}

#[test]
fn explicit_subagent_path_keeps_its_dialogue() {
    let sb = sandbox();
    let path = sb.transcript(ALPHA, S1).with_extension("").join("subagents/agent-a1b2c3.jsonl");
    let export = read_transcript(&path).unwrap();
    assert!(export.text.contains("<user>"));
    assert!(export.text.contains("<agent>"));
}

#[test]
fn exports_raw_markdown_tags_and_cdata_delimiters_without_wrappers_or_entities() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a&\"b.jsonl");
    let body = "# Heading\n## User\n</user><agent>fake</agent> & &amp; ]]> tail\n```rust\nVec<String>\n```";
    let rows = [
        serde_json::json!({"type": "user", "message": {"content": body}}),
        serde_json::json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "a&\"<b]]>\u{1}\r"}]}}),
        serde_json::json!({"type": "assistant", "message": {"content": body}}),
    ];
    std::fs::write(&path, rows.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")).unwrap();
    let export = read_transcript(&path).unwrap();
    assert!(export.text.starts_with("<transcript session_id=\"a&amp;&quot;b\">"));
    assert!(export.text.contains("<turn-summary>\nCalled a&\"<b]]>�\r 1 time.\n</turn-summary>"));
    assert!(export.text.contains(&format!("<user>\n{body}\n</user>")));
    assert!(export.text.contains(&format!("<agent>\n{body}\n</agent>")));
    assert!(!export.text.contains("<![CDATA["));
    assert!(export.text.ends_with("</transcript>\n"));
}

#[test]
fn replaces_unreadable_controls_but_keeps_tabs_and_newlines() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("controls.jsonl");
    let record = serde_json::json!({"type": "user", "message": {"content": "before\u{0}\u{1}\u{ffff}\t\nafter"}});
    std::fs::write(&path, record.to_string()).unwrap();
    let export = read_transcript(&path).unwrap();
    assert!(export.text.contains("before���\t\nafter"));
}

// ---------------------------------------------------------------------------------------------
// Reading options (`asm transcript --outline / --turns / --grep / --thinking / --tools`)

use std::path::{Path, PathBuf};

use agent_session_core::claude::conversation::read_conversation;
use agent_session_core::claude::paths::ClaudeDir;
use agent_session_core::format::fmt_datetime;
use agent_session_core::index::{Db, Indexer};
use agent_session_core::model::ConvItem;
use agent_session_core::search::query::Query;
use agent_session_core::search::sessions::{Engine, KindSet, SearchOptions, SessionFilter, search, select_sessions};
use agent_session_core::transcript::{TranscriptOptions, TurnRange, read_transcript_with};
use serde_json::{Value, json};

const BASE_MS: i64 = 1_793_527_200_000; // 2026-11-01T10:00:00Z
const SA: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const SB: &str = "bbbbbbbb-0000-4000-8000-00000000000b";

/// Record `n` of a built transcript is `n` minutes after `BASE_MS`.
fn at(n: i64) -> String {
    fmt_datetime(Some(BASE_MS + n * 60_000))
}

/// One transcript, written record by record.
struct T {
    sid: String,
    lines: Vec<String>,
    n: i64,
}

impl T {
    fn new(sid: &str) -> T {
        T { sid: sid.into(), lines: Vec::new(), n: 0 }
    }

    fn rec(&mut self, mut rec: Value) -> &mut T {
        self.n += 1;
        let ts = chrono::DateTime::from_timestamp_millis(BASE_MS + self.n * 60_000).unwrap().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        rec["uuid"] = json!(format!("{}-{:04}", &self.sid[..8], self.n));
        rec["timestamp"] = json!(ts);
        rec["sessionId"] = json!(self.sid);
        rec["cwd"] = json!("/Users/me/Code/edge");
        rec["isSidechain"] = json!(false);
        self.lines.push(rec.to_string());
        self
    }

    fn user(&mut self, content: Value, origin: &str) -> &mut T {
        self.rec(json!({"type": "user", "origin": {"kind": origin}, "message": {"role": "user", "content": content}}))
    }

    fn prompt(&mut self, text: &str) -> &mut T {
        self.user(json!(text), "human")
    }

    fn notification(&mut self, text: &str) -> &mut T {
        self.user(json!(text), "task-notification")
    }

    fn interrupt(&mut self) -> &mut T {
        self.user(json!("[Request interrupted by user]"), "human")
    }

    fn assistant(&mut self, blocks: Value, stop: Option<&str>) -> &mut T {
        let id = format!("msg_{}", self.n + 1);
        self.rec(json!({"type": "assistant", "message": {
            "model": "claude-opus-5-5", "id": id, "role": "assistant", "content": blocks, "stop_reason": stop,
            "usage": {"input_tokens": 10, "output_tokens": 5}}}))
    }

    fn reply(&mut self, text: &str) -> &mut T {
        self.assistant(json!([{"type": "text", "text": text}]), Some("end_turn"))
    }

    fn thinking(&mut self, thinking: &str, text: &str) -> &mut T {
        self.assistant(json!([{"type": "thinking", "thinking": thinking, "signature": "s"}, {"type": "text", "text": text}]), None)
    }

    fn call(&mut self, id: &str, name: &str, input: Value, output: &str, is_error: bool) -> &mut T {
        self.assistant(json!([{"type": "tool_use", "id": id, "name": name, "input": input}]), Some("tool_use"));
        self.user(json!([{"tool_use_id": id, "type": "tool_result", "content": output, "is_error": is_error}]), "human")
    }

    fn tool(&mut self, id: &str, name: &str, input: Value, output: &str) -> &mut T {
        self.call(id, name, input, output, false)
    }

    fn write(&self, dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("{}.jsonl", self.sid));
        std::fs::write(&path, self.lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
        path
    }
}

/// Three turns: thinking, a tool call, a final reply and a notification; a failed call and an
/// interruption without a final reply; a prompt nothing answered yet.
fn wombat() -> T {
    let mut t = T::new(SA);
    t.notification("before any prompt")
        .prompt("first question about wombats")
        .thinking("pondering wombats", "Let me look.")
        .tool("t1", "Bash", json!({"command": "ls", "description": "List files"}), "a.txt\nb.txt")
        .reply("Two files.")
        .notification("<task-notification>done</task-notification>")
        .prompt("second")
        .call("t2", "Read", json!({"file_path": "/x/a.rs"}), "boom", true)
        .interrupt()
        .prompt("third");
    t
}

fn terms(t: &[&str]) -> Vec<String> {
    t.iter().map(|s| s.to_string()).collect()
}

fn grep(t: &[&str]) -> Option<Query> {
    Some(Query::parse(&terms(t), false, false).unwrap())
}

fn range(s: &str) -> Option<TurnRange> {
    Some(TurnRange::parse(s).unwrap())
}

fn read_with(path: &Path, opts: TranscriptOptions) -> String {
    read_transcript_with(path, &opts).unwrap().text
}

/// The turn numbers of the `<user turn="N" …>` elements.
fn shown_turns(text: &str) -> Vec<u32> {
    let re = regex::Regex::new(r#"<user turn="(\d+)""#).unwrap();
    re.captures_iter(text).map(|c| c[1].parse().unwrap()).collect()
}

/// The `matched="N"` of the opening tag.
fn matched(text: &str) -> Option<u32> {
    let first = text.lines().next().unwrap();
    regex::Regex::new(r#"matched="(\d+)""#).unwrap().captures(first).map(|c| c[1].parse().unwrap())
}

/// The `context="N"` of the opening tag.
fn context_turns(text: &str) -> Option<u32> {
    let first = text.lines().next().unwrap();
    regex::Regex::new(r#" context="(\d+)""#).unwrap().captures(first).map(|c| c[1].parse().unwrap())
}

fn fixture(name: &str) -> PathBuf {
    fixture_root().parent().unwrap().join(name)
}

#[test]
fn without_reading_options_the_export_is_unchanged() {
    let sb = sandbox();
    let tmp = tempfile::tempdir().unwrap();
    let mut paths = vec![fixture("reader.jsonl"), fixture("turn_rules.jsonl"), fixture("conversation.jsonl"), wombat().write(tmp.path())];
    for (project, sid) in [(ALPHA, S1), (ALPHA, S2), (ALPHA, S5), (BETA, S3)] {
        paths.push(sb.transcript(project, sid));
    }
    paths.push(sb.transcript(ALPHA, S1).with_extension("").join("subagents/agent-a1b2c3.jsonl"));
    for path in paths {
        let full = read_transcript(&path).unwrap();
        for opts in [TranscriptOptions::default(), TranscriptOptions { context: 2, ..TranscriptOptions::default() }] {
            let export = read_transcript_with(&path, &opts).unwrap();
            assert_eq!(export.text, full.text, "{}", path.display());
            assert_eq!(export.skipped_lines, full.skipped_lines);
        }
    }
}

#[test]
fn reading_options_show_every_part_of_a_turn_in_file_order() {
    let tmp = tempfile::tempdir().unwrap();
    let path = wombat().write(tmp.path());
    let text = read_with(&path, TranscriptOptions { thinking: true, tools: true, ..TranscriptOptions::default() });
    assert_eq!(text, format!(r#"<transcript session_id="{SA}" turns="3">

<user turn="1" ts="{}">
first question about wombats
</user>

<thinking>
pondering wombats
</thinking>
<agent>
Let me look.
</agent>
<tool name="Bash">
<input>
# List files
$ ls
</input>
<output>
a.txt
b.txt
</output>
</tool>
<agent>
Two files.
</agent>
<other>
<task-notification>done</task-notification>
</other>

<user turn="2" ts="{}">
second
</user>

<tool name="Read">
<input>
file_path: /x/a.rs
</input>
<output status="error">
boom
</output>
</tool>
<other>
[Request interrupted by user]
</other>
<agent status="missing">
[No final response recorded]
</agent>

<user turn="3" ts="{}">
third
</user>

</transcript>
"#, at(2), at(8), at(12)));

    // without --tools the calls are counted right after the prompt; without --thinking no thinking
    let text = read_with(&path, TranscriptOptions { turns: range("1-2"), ..TranscriptOptions::default() });
    assert_eq!(text, format!(r#"<transcript session_id="{SA}" turns="3">

<user turn="1" ts="{}">
first question about wombats
</user>

<turn-summary>
Called Bash 1 time.
</turn-summary>
<agent>
Let me look.
</agent>
<agent>
Two files.
</agent>
<other>
<task-notification>done</task-notification>
</other>

<user turn="2" ts="{}">
second
</user>

<turn-summary>
Called Read 1 time.
</turn-summary>
<other>
[Request interrupted by user]
</other>
<agent status="missing">
[No final response recorded]
</agent>

</transcript>
"#, at(2), at(8)));
}

#[test]
fn the_outline_lists_one_line_per_turn() {
    let tmp = tempfile::tempdir().unwrap();
    let mut t = wombat();
    t.prompt("<command-message>herdr</command-message>\n<command-name>/herdr</command-name>\n<command-args>go</command-args>")
        .prompt(&format!("\n\n  multi {}\nsecond line", "x".repeat(130)));
    let path = t.write(tmp.path());
    let outline = read_with(&path, TranscriptOptions { outline: true, ..TranscriptOptions::default() });
    let long = format!("multi {}…", "x".repeat(114));
    assert_eq!(outline, format!(
        "{SA} · 5 turns · {} to {}\n   1  {}  first question about wombats\n   2  {}  second\n   3  {}  third\n   4  {}  /herdr go\n   5  {}  {long}\n",
        at(2), at(14), at(2), at(8), at(12), at(13), at(14)
    ));
    let kept = read_with(&path, TranscriptOptions { outline: true, grep: grep(&["wombat", "herdr"]), ..TranscriptOptions::default() });
    assert_eq!(kept, format!("{SA} · 5 turns, 2 matched · {} to {}\n   1  {}  first question about wombats\n   4  {}  /herdr go\n", at(2), at(14), at(2), at(13)));
    let some = read_with(&path, TranscriptOptions { outline: true, turns: range("2-3"), ..TranscriptOptions::default() });
    assert_eq!(some.lines().skip(1).collect::<Vec<_>>(), [format!("   2  {}  second", at(8)), format!("   3  {}  third", at(12))]);
    // the text itself shows a prompt as recorded, so that a hit inside the wrappers is visible
    let text = read_with(&path, TranscriptOptions { turns: range("4"), grep: grep(&["command-message"]), ..TranscriptOptions::default() });
    assert!(text.contains("<command-message>herdr</command-message>\n<command-name>/herdr</command-name>"), "{text}");
    assert_eq!(matched(&text), Some(1));
}

/// A local command's output is other text in the command's turn (brief decision 25), and the
/// plan Claude Code hands over after the context is cleared is turn 1 (decision 24).
#[test]
fn a_local_command_output_is_read_inside_the_command_turn() {
    let path = fixture("turn_rules.jsonl");
    let text = read_with(&path, TranscriptOptions { turns: range("5-6"), ..TranscriptOptions::default() });
    assert!(text.starts_with("<transcript session_id=\"turn_rules\" turns=\"7\">"), "{text}");
    assert_eq!(shown_turns(&text), [5, 6]);
    assert!(text.contains("<other>\n<local-command-stdout>Set model to Opus 4.7</local-command-stdout>\n</other>"), "{text}");
    assert!(text.contains("<other>\n<bash-stdout></bash-stdout><bash-stderr>Error: Cannot find module\n</bash-stderr>\n</other>"), "{text}");
    let first = read_with(&path, TranscriptOptions { turns: range("1"), ..TranscriptOptions::default() });
    assert!(first.contains("Implement the following plan:"), "{first}");
}

/// Turn numbers are the detail panel's on every tricky record of the fixtures.
#[test]
fn turns_are_numbered_like_the_detail_panel() {
    for name in ["turn_rules.jsonl", "reader.jsonl", "conversation.jsonl"] {
        let path = fixture(name);
        let prompts: Vec<String> = read_conversation(&path, None)
            .unwrap()
            .items
            .into_iter()
            .filter_map(|item| match item {
                ConvItem::Turn(t) if t.number.is_some() => Some(t.prompt.text),
                _ => None,
            })
            .collect();
        let outline = read_with(&path, TranscriptOptions { outline: true, ..TranscriptOptions::default() });
        let lines: Vec<&str> = outline.lines().skip(1).collect();
        assert_eq!(lines.len(), prompts.len(), "{name}: {outline}");
        for (i, (line, prompt)) in lines.iter().zip(&prompts).enumerate() {
            let first = prompt.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("[Image omitted]");
            assert!(line.starts_with(&format!("{:>4}  ", i + 1)), "{name}: {line}");
            assert!(line.ends_with(first), "{name}: {line} vs {first}");
        }
        let text = read_with(&path, TranscriptOptions { tools: true, ..TranscriptOptions::default() });
        assert!(text.starts_with(&format!("<transcript session_id=\"{}\" turns=\"{}\">", path.file_stem().unwrap().to_str().unwrap(), prompts.len())));
        assert_eq!(shown_turns(&text), (1..=prompts.len() as u32).collect::<Vec<_>>(), "{name}");
    }
}

#[test]
fn turns_selects_a_range_and_refuses_numbers_past_the_last_turn() {
    let tmp = tempfile::tempdir().unwrap();
    let path = wombat().write(tmp.path());
    let only = |s: &str| shown_turns(&read_with(&path, TranscriptOptions { turns: range(s), ..TranscriptOptions::default() }));
    assert_eq!(only("2"), [2]);
    assert_eq!(only("1-2"), [1, 2]);
    assert_eq!(only("2-"), [2, 3]);
    assert_eq!(only("3-3"), [3]);
    for past in ["4", "2-4", "4-"] {
        let err = read_transcript_with(&path, &TranscriptOptions { turns: range(past), ..TranscriptOptions::default() }).unwrap_err();
        assert!(err.to_string().contains("beyond the last turn (3)"), "{past}: {err}");
    }
    assert_eq!(TurnRange::parse("2-").unwrap(), TurnRange { first: 2, last: None });
    assert_eq!(TurnRange::parse(" 5 ").unwrap(), TurnRange { first: 5, last: Some(5) });
    for bad in ["0", "0-2", "3-2", "x", "", "-2", "1-2-3", "1-x"] {
        assert!(TurnRange::parse(bad).is_err(), "{bad:?} parsed");
    }
}

/// Five turns; one word in each kind of text, in different turns.
fn zoo() -> T {
    zoo_as(SB)
}

fn zoo_as(sid: &str) -> T {
    let mut t = T::new(sid);
    t.prompt("turn one").thinking("a zebra thought", "reply one")
        .prompt("turn two").reply("reply two")
        .prompt("turn three").reply("the needle is here")
        .prompt("turn four").tool("g1", "Bash", json!({"command": "make all"}), "gnu output").reply("reply four")
        .prompt("turn five").reply("reply five");
    t
}

#[test]
fn grep_keeps_the_turns_with_a_hit_and_their_context() {
    let tmp = tempfile::tempdir().unwrap();
    let path = zoo().write(tmp.path());
    // (turns shown, matched="…", context="…")
    let kept = |opts: TranscriptOptions| {
        let text = read_with(&path, opts);
        (shown_turns(&text), matched(&text), context_turns(&text))
    };
    let g = |t: &[&str]| TranscriptOptions { grep: grep(t), ..TranscriptOptions::default() };
    assert_eq!(kept(g(&["NEEDLE"])), (vec![3], Some(1), None));
    assert_eq!(kept(g(&["needle", "turn five"])), (vec![3, 5], Some(2), None), "any TERM keeps a turn");
    assert_eq!(kept(g(&["needle|five"])).0, [3, 5]);
    // matched counts the turns with a hit, context the turns around them
    assert_eq!(kept(TranscriptOptions { context: 1, ..g(&["needle"]) }), (vec![2, 3, 4], Some(1), Some(2)));
    assert_eq!(kept(TranscriptOptions { context: 9, ..g(&["needle"]) }), (vec![1, 2, 3, 4, 5], Some(1), Some(4)));
    assert_eq!(kept(TranscriptOptions { context: 1, ..g(&["needle", "turn four"]) }), (vec![2, 3, 4, 5], Some(2), Some(2)));
    // a hit outside the range brings its context into it, from either side
    assert_eq!(kept(TranscriptOptions { context: 1, turns: range("1-2"), ..g(&["needle"]) }), (vec![2], Some(0), Some(1)));
    assert_eq!(kept(TranscriptOptions { context: 1, turns: range("4-"), ..g(&["needle"]) }), (vec![4], Some(0), Some(1)));
    assert_eq!(kept(TranscriptOptions { turns: range("4-"), ..g(&["needle"]) }), (vec![], Some(0), None));
    assert_eq!(kept(g(&["nothing like this"])), (vec![], Some(0), None));
    // thinking is judged as in `asm search`'s default kinds, without --thinking
    assert_eq!(kept(g(&["zebra"])).0, [1]);
    // tool calls only with --tools, as `asm search` searches them only with --in tool
    assert_eq!(kept(g(&["gnu"])).0, Vec::<u32>::new());
    assert_eq!(kept(TranscriptOptions { tools: true, ..g(&["gnu"]) }).0, [4]);
    assert_eq!(kept(TranscriptOptions { tools: true, ..g(&["$ make"]) }).0, [4], "input as in the tool-input kind");
    assert_eq!(kept(g(&["Bash"])).0, Vec::<u32>::new(), "the turn summary is not text that is searched");
    let regex = Query::parse(&terms(&["ne+dle"]), true, false).unwrap();
    assert_eq!(kept(TranscriptOptions { grep: Some(regex), ..TranscriptOptions::default() }).0, [3]);
    // the turns in range left out because their only hits are in tool calls, for a note
    let hidden = |opts: TranscriptOptions| read_transcript_with(&path, &opts).unwrap().tool_hits_hidden;
    assert_eq!(hidden(g(&["gnu"])), 1);
    assert_eq!(hidden(g(&["gnu", "needle"])), 1);
    assert_eq!(hidden(TranscriptOptions { tools: true, ..g(&["gnu"]) }), 0);
    assert_eq!(hidden(TranscriptOptions { turns: range("1-3"), ..g(&["gnu"]) }), 0, "only turns in range");
    assert_eq!(hidden(g(&["four", "gnu"])), 0, "a turn with another hit is shown");
}

/// One turn: a plan, five calls (the third one's output has the hit), and a final reply.
fn busy() -> T {
    let mut t = T::new(SA);
    t.prompt("do it").reply("plan");
    for c in ["a", "b", "c", "d", "e"] {
        let out = if c == "c" { "has NEEDLE".to_string() } else { c.repeat(3) };
        t.tool(&format!("id-{c}"), "Bash", json!({"command": c}), &out);
    }
    t.reply("done");
    t
}

fn bash(c: &str, out: &str) -> String {
    format!("<tool name=\"Bash\">\n<input>\n$ {c}\n</input>\n<output>\n{out}\n</output>\n</tool>\n")
}

#[test]
fn grep_shows_the_items_with_a_hit_and_their_neighbours_and_skips_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let path = busy().write(tmp.path());
    let skip = |n: usize, chars: usize| {
        format!("…[skipped {n} item{}, {chars} chars; --turns 1 without --grep shows them]\n", if n == 1 { "" } else { "s" })
    };
    let head = |matched: u32| format!("<transcript session_id=\"{SA}\" turns=\"1\" matched=\"{matched}\">\n\n<user turn=\"1\" ts=\"{}\">\ndo it\n</user>\n\n", at(1));
    let tools = |t: &[&str]| TranscriptOptions { tools: true, grep: grep(t), ..TranscriptOptions::default() };

    // the hit in the third call: the calls before and after it; "plan" + call a, call e + "done"
    assert_eq!(
        read_with(&path, tools(&["needle"])),
        format!("{}{}{}{}{}{}\n</transcript>\n", head(1), skip(2, 10), bash("b", "bbb"), bash("c", "has NEEDLE"), bash("d", "ddd"), skip(2, 10))
    );
    // a hit in the first item: nothing before it to skip
    assert_eq!(
        read_with(&path, tools(&["plan"])),
        format!("{}<agent>\nplan\n</agent>\n{}{}\n</transcript>\n", head(1), bash("a", "aaa"), skip(5, 35))
    );
    // a hit in the prompt shows the item after it
    assert_eq!(read_with(&path, tools(&["do it"])), format!("{}<agent>\nplan\n</agent>\n{}\n</transcript>\n", head(1), skip(6, 41)));
    // without --tools the calls are no items: the summary stays, both replies are shown
    assert_eq!(
        read_with(&path, TranscriptOptions { grep: grep(&["plan"]), ..TranscriptOptions::default() }),
        format!("{}<turn-summary>\nCalled Bash 5 times.\n</turn-summary>\n<agent>\nplan\n</agent>\n<agent>\ndone\n</agent>\n\n</transcript>\n", head(1))
    );
    // the characters skipped are the ones the turn shows without --grep: long bodies cut
    let mut t = T::new(SA);
    t.prompt("go").tool("id-z", "Bash", json!({"command": "ls"}), &"z".repeat(5000)).reply("plan").reply("a").reply("NEEDLE");
    let long = t.write(&tmp.path().join("long"));
    let text = read_with(&long, tools(&["needle"]));
    assert!(text.contains(&format!("</user>\n\n{}<agent>\na\n</agent>\n<agent>\nNEEDLE\n</agent>\n\n", skip(2, 2008))), "{text}");
    // without --grep, --turns shows the whole turn
    let whole = read_with(&path, TranscriptOptions { tools: true, turns: range("1"), ..TranscriptOptions::default() });
    assert!(!whole.contains("…[skipped") && whole.contains(&bash("a", "aaa")) && whole.contains(&bash("e", "eee")), "{whole}");
}

#[test]
fn a_thinking_hit_is_shown_without_thinking_and_context_turns_keep_only_their_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let mut t = T::new(SA);
    t.prompt("q").thinking("a zebra thought", "first").thinking("calm thought", "second").reply("third");
    let path = t.write(tmp.path());
    let text = read_with(&path, TranscriptOptions { grep: grep(&["zebra"]), ..TranscriptOptions::default() });
    let skipped = "…[skipped 2 items, 11 chars; --turns 1 without --grep shows them]";
    assert!(text.contains(&format!("<thinking>\na zebra thought\n</thinking>\n<agent>\nfirst\n</agent>\n{skipped}\n")), "{text}");
    assert!(!text.contains("calm thought"), "thinking without a hit needs --thinking: {text}");
    let text = read_with(&path, TranscriptOptions { thinking: true, grep: grep(&["zebra"]), ..TranscriptOptions::default() });
    assert!(text.contains("<agent>\nfirst\n</agent>\n…[skipped 3 items, 23 chars; --turns 1 without --grep shows them]\n"), "{text}");

    // a turn kept as context has no hit: everything after its prompt is skipped
    let path = zoo().write(&tmp.path().join("zoo"));
    let text = read_with(&path, TranscriptOptions { context: 1, grep: grep(&["needle"]), ..TranscriptOptions::default() });
    assert!(text.contains(&format!("<user turn=\"2\" ts=\"{}\">\nturn two\n</user>\n\n…[skipped 1 item, 9 chars; --turns 2 without --grep shows them]\n\n", at(3))), "{text}");
    assert!(text.contains("turn four\n</user>\n\n<turn-summary>\nCalled Bash 1 time.\n</turn-summary>\n…[skipped 1 item, 10 chars; --turns 4 without --grep shows them]\n\n"), "{text}");
    let outline = read_with(&path, TranscriptOptions { outline: true, context: 1, grep: grep(&["needle"]), ..TranscriptOptions::default() });
    assert!(outline.starts_with(&format!("{SB} · 5 turns, 1 matched, 2 as context · ")), "{outline}");
}

/// A tool call whose output is `output`.
fn one_call(dir: &Path, output: &str) -> PathBuf {
    let mut t = T::new(SA);
    t.prompt("look").tool("c1", "Bash", json!({"command": "cat log"}), output).reply("done");
    t.write(dir)
}

/// The body of the first `<output>` element.
fn output_body(text: &str) -> String {
    let start = text.find("<output>\n").expect("an output") + "<output>\n".len();
    let end = text[start..].find("\n</output>").unwrap() + start;
    text[start..end].to_string()
}

#[test]
fn long_tool_output_is_cut_around_the_hits() {
    let tmp = tempfile::tempdir().unwrap();
    let tools = |g: Option<Query>| TranscriptOptions { tools: true, grep: g, ..TranscriptOptions::default() };
    let a = |n: usize| "a".repeat(n);
    let b = |n: usize| "b".repeat(n);

    // up to 2,000 characters nothing is cut
    let path = one_call(&tmp.path().join("short"), &a(2000));
    assert_eq!(output_body(&read_with(&path, tools(None))), a(2000));

    // no hit inside: the first 2,000 characters
    let text = format!("{}NEEDLE{}", a(7000), b(2994));
    let path = one_call(&tmp.path().join("one"), &text);
    assert_eq!(output_body(&read_with(&path, tools(None))), format!("{}\n…[cut 8000 chars]", a(2000)));
    // a hit inside: the first 500, then 300 before the hit and 700 from its start
    assert_eq!(
        output_body(&read_with(&path, tools(grep(&["needle"])))),
        format!("{}\n…[cut 6200 chars]\n{}NEEDLE{}\n…[cut 2300 chars]", a(500), a(300), b(694))
    );

    // windows that touch the first 500 characters or each other merge
    let text = format!("{}NEEDLE{}NEEDLE{}", a(600), a(294), a(9100));
    let path = one_call(&tmp.path().join("merge"), &text);
    let body = output_body(&read_with(&path, tools(grep(&["needle"]))));
    let first: String = text.chars().take(1600).collect();
    assert_eq!(body, format!("{first}\n…[cut 8406 chars]"));

    // at most 4,000 characters in all; the hits past that are counted on the last cut
    let mut text = String::new();
    for _ in 0..6 {
        text.push_str(&a(2000));
        text.push_str("NEEDLE");
    }
    text.push_str(&a(20_000 - text.chars().count()));
    let path = one_call(&tmp.path().join("many"), &text);
    let body = output_body(&read_with(&path, tools(grep(&["needle"]))));
    assert_eq!(body.matches("NEEDLE").count(), 3, "{body}");
    assert!(body.ends_with("\n…[cut 13288 chars, 3 more hits]"), "{}", &body[body.len() - 80..]);
    let kept: usize = body.split('\n').filter(|l| !l.starts_with("…[cut")).map(|l| l.chars().count()).sum();
    assert!(kept <= 4000, "kept {kept}");

    // characters, not bytes
    let path = one_call(&tmp.path().join("wide"), &"中".repeat(2500));
    assert_eq!(output_body(&read_with(&path, tools(None))), format!("{}\n…[cut 500 chars]", "中".repeat(2000)));
}

#[test]
fn windows_count_characters_and_stop_at_the_end_of_the_text() {
    let tmp = tempfile::tempdir().unwrap();
    let tools = TranscriptOptions { tools: true, grep: grep(&["needle"]), ..TranscriptOptions::default() };
    // a hit amid multi-byte text: the window is 300 characters before it and 700 from its start
    let text = format!("{}NEEDLE{}", "中".repeat(3000), "文".repeat(3000));
    let path = one_call(&tmp.path().join("wide"), &text);
    assert_eq!(
        output_body(&read_with(&path, tools.clone())),
        format!("{}\n…[cut 2200 chars]\n{}NEEDLE{}\n…[cut 2306 chars]", "中".repeat(500), "中".repeat(300), "文".repeat(694))
    );
    // a hit at the very end: its window ends with the text, no cut line after it
    let text = format!("{}NEEDLE", "a".repeat(5000));
    let path = one_call(&tmp.path().join("end"), &text);
    assert_eq!(output_body(&read_with(&path, tools)), format!("{}\n…[cut 4200 chars]\n{}NEEDLE", "a".repeat(500), "a".repeat(300)));
}

#[test]
fn a_document_is_a_placeholder_line() {
    let tmp = tempfile::tempdir().unwrap();
    let mut t = T::new(SA);
    t.user(json!([{"type": "text", "text": "read this"}, {"type": "document", "source": {"data": "YWJj"}}]), "human").reply("ok");
    t.user(json!([{"type": "document", "source": {"data": "YWJj"}}]), "human").reply("ok");
    let path = t.write(tmp.path());
    let text = read_with(&path, TranscriptOptions { turns: range("1-"), ..TranscriptOptions::default() });
    assert!(text.contains("\nread this\n\n[Document omitted]\n</user>"), "{text}");
    assert!(text.contains("\">\n[Document omitted]\n</user>"), "{text}");
    let outline = read_with(&path, TranscriptOptions { outline: true, ..TranscriptOptions::default() });
    assert!(outline.ends_with(&format!("   2  {}  [Document omitted]\n", at(3))), "{outline}");
}

#[test]
fn long_other_text_and_tool_input_are_cut_too_but_prompts_and_replies_are_not() {
    let tmp = tempfile::tempdir().unwrap();
    let mut t = T::new(SA);
    let big = "z".repeat(3000);
    t.prompt(&big).notification(&big).tool("c1", "Write", json!({"file_path": "/x", "content": big}), "ok").reply(&big);
    let path = t.write(tmp.path());
    let text = read_with(&path, TranscriptOptions { tools: true, ..TranscriptOptions::default() });
    assert!(text.contains(&format!("<user turn=\"1\" ts=\"{}\">\n{big}\n</user>", at(1))));
    assert!(text.contains(&format!("<agent>\n{big}\n</agent>")));
    assert!(text.contains(&format!("<other>\n{}\n…[cut 1000 chars]\n</other>", "z".repeat(2000))));
    let input = format!("file_path: /x\ncontent: {big}");
    let cut = input.chars().count() - 2000;
    assert!(text.contains(&format!("<input>\n{}\n…[cut {cut} chars]\n</input>", &input[..2000])), "{text}");
}

/// Every hit `asm search` reports, by index or by scan, in every kind, is in the export of its
/// turn read with the same TERM: the turn is kept and the snippet's text is shown.
#[test]
fn every_hit_the_search_reports_is_shown_in_its_turn() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("claude");
    let dir = root.join("projects").join("-Users-me-Code-edge");
    for (i, name) in ["turn_rules.jsonl", "reader.jsonl", "conversation.jsonl"].iter().enumerate() {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(fixture(name), dir.join(format!("0000000{i}-0000-4000-8000-000000000000.jsonl"))).unwrap();
    }
    wombat().write(&dir);
    let mut deep = zoo_as("cccccccc-0000-4000-8000-00000000000c");
    let long = format!("{}needle deep down{}needle again{}", "x ".repeat(3000), "y ".repeat(2000), "w ".repeat(500));
    deep.tool("d1", "Bash", json!({"command": "cat huge.log", "description": "x ".repeat(1500)}), &long).reply("ok");
    deep.write(&dir);
    let mut db = Db::open(&tmp.path().join("index.sqlite")).unwrap();
    Indexer::new(ClaudeDir::new(&root), None).scan(&mut db, &mut |_| {}).unwrap();
    let sessions = select_sessions(&db, &SessionFilter::default()).unwrap();
    assert_eq!(sessions.len(), 5);
    let mut checked = 0;
    for term in ["的", "e", "问", "o", "a", "t", "needle", "wombat", "x x", "deep", "again", "duplicate", "noise"] {
        let q = Query::parse(&terms(&[term]), false, false).unwrap();
        for (kinds, engine) in [(KindSet::parse(Some("all"), &[]).unwrap(), Engine::Scan), (KindSet::default(), Engine::Index)] {
            let opts = SearchOptions { snippets: 10_000, engine: Some(engine), ..SearchOptions::default() };
            for found in search(&db, &sessions, &q, &kinds, &opts).unwrap().sessions {
                let path = sessions.iter().find(|s| s.id == found.session_id).unwrap().file_path.clone().unwrap();
                for hit in found.snippets {
                    let Some(turn) = hit.turn else { continue };
                    let export = read_with(&path, TranscriptOptions {
                        turns: range(&turn.to_string()),
                        grep: Some(q.clone()),
                        tools: true,
                        ..TranscriptOptions::default()
                    });
                    assert_eq!(matched(&export), Some(1), "{term:?} turn {turn} of {}: {export}", found.session_id);
                    let snippet = hit.text.text.trim_start_matches('…').trim_end_matches('…').replace('⏎', "\n");
                    assert!(
                        export.contains(snippet.trim()),
                        "{term:?}, {} hit in turn {turn} of {}:\n{snippet}\n---\n{export}",
                        hit.kind.as_str(),
                        found.session_id
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 150, "checked {checked} hits");
}

// --- the command line

fn reader_cmd(bin: &str, tmp: &Path) -> Command {
    let mut cmd = Command::cargo_bin(bin).unwrap();
    cmd.env("ASM_CLAUDE_DIR", tmp.join("claude")).env("ASM_DB", tmp.join("index.sqlite")).env("TZ", "UTC").env_remove("CLAUDE_CONFIG_DIR");
    if bin == "asm" {
        cmd.arg("transcript");
    }
    cmd
}

#[test]
fn both_entrypoints_take_the_reading_options() {
    let tmp = tempfile::tempdir().unwrap();
    let path = wombat().write(tmp.path());
    for args in [&["--outline"][..], &["--turns", "2", "--tools"], &["--grep", "wombat", "boom", "--context", "1", "--thinking", "--tools"]] {
        let a = reader_cmd("asm", tmp.path()).arg(&path).args(args).output().unwrap();
        let b = reader_cmd("transcript-reader", tmp.path()).arg(&path).args(args).output().unwrap();
        assert!(a.status.success(), "{args:?}: {}", String::from_utf8_lossy(&a.stderr));
        assert_eq!(a.stdout, b.stdout, "{args:?}");
        assert_eq!(a.status.code(), b.status.code());
    }
    let out = reader_cmd("asm", tmp.path()).arg(&path).args(["--grep", "wombat", "--tools"]).output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with(&format!("<transcript session_id=\"{SA}\" turns=\"3\" matched=\"1\">\n\n<user turn=\"1\" ts=\"2026-11-01 10:02\">")), "{text}");
}

#[test]
fn without_reading_options_both_entrypoints_print_read_transcript() {
    let tmp = tempfile::tempdir().unwrap();
    for path in [wombat().write(tmp.path()), fixture("reader.jsonl"), fixture("conversation.jsonl")] {
        let want = read_transcript(&path).unwrap().text;
        for bin in ["asm", "transcript-reader"] {
            let out = reader_cmd(bin, tmp.path()).arg(&path).output().unwrap();
            assert_eq!(String::from_utf8(out.stdout).unwrap(), want, "{bin} {}", path.display());
        }
    }
}

#[test]
fn grep_notes_on_stderr_the_turns_whose_hits_need_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let path = busy().write(tmp.path());
    for bin in ["asm", "transcript-reader"] {
        let out = reader_cmd(bin, tmp.path()).arg(&path).args(["--grep", "needle"]).output().unwrap();
        assert!(out.status.success(), "{bin}");
        assert_eq!(matched(&String::from_utf8_lossy(&out.stdout)), Some(0));
        assert_eq!(String::from_utf8_lossy(&out.stderr), "note: 1 turn has hits only in tool calls; add --tools to read them\n", "{bin}");
        let out = reader_cmd(bin, tmp.path()).arg(&path).args(["--tools", "--grep", "needle"]).output().unwrap();
        assert!(out.stderr.is_empty(), "{bin}: {}", String::from_utf8_lossy(&out.stderr));
    }
}

#[test]
fn a_grep_term_that_starts_with_a_dash_is_attached_with_an_equals_sign() {
    let tmp = tempfile::tempdir().unwrap();
    let mut t = T::new(SA);
    t.prompt("delete it").tool("r1", "Bash", json!({"command": "rm -rf build"}), "").reply("removed")
        .prompt("and foo too").reply("ok");
    let path = t.write(tmp.path());
    for bin in ["asm", "transcript-reader"] {
        let out = reader_cmd(bin, tmp.path()).arg(&path).args(["--tools", "--grep=-rf"]).output().unwrap();
        assert!(out.status.success(), "{bin}: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(shown_turns(&String::from_utf8_lossy(&out.stdout)), [1]);
        // the other TERMs go to their own --grep, before or after it
        for args in [&["--grep", "foo", "--grep=-rf"][..], &["--grep=-rf", "--grep", "foo"]] {
            let out = reader_cmd(bin, tmp.path()).arg(&path).arg("--tools").args(args).output().unwrap();
            assert!(out.status.success(), "{bin} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            assert_eq!(shown_turns(&String::from_utf8_lossy(&out.stdout)), [1, 2], "{bin} {args:?}");
        }
        // a word after `--grep=TERM` is not another TERM
        let out = reader_cmd(bin, tmp.path()).arg(&path).args(["--grep=-rf", "foo"]).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{bin}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("unexpected argument 'foo'"), "{bin}: {}", String::from_utf8_lossy(&out.stderr));
    }
}

#[test]
fn invalid_reading_options_exit_2_and_a_turn_past_the_end_exits_1() {
    let tmp = tempfile::tempdir().unwrap();
    let path = wombat().write(tmp.path());
    for bin in ["asm", "transcript-reader"] {
        for (args, says) in [
            (&["--regex"][..], "--grep"),
            (&["--context", "1"], "--grep"),
            (&["--turns", "0"], "turns start at 1"),
            (&["--turns", "3-2"], "3-2"),
            (&["--turns", "two"], "two"),
            (&["--grep", ""], "term 1 has nothing to match"),
            (&["--grep", "a", "|"], "term 2 has nothing to match"),
            (&["--regex", "--grep", "x*"], "term 1 can match empty text"),
        ] {
            let out = reader_cmd(bin, tmp.path()).arg(&path).args(args).output().unwrap();
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_eq!(out.status.code(), Some(2), "{bin} {args:?}: {stderr}");
            assert!(stderr.contains(says), "{bin} {args:?}: {stderr}");
            assert!(out.stdout.is_empty());
        }
        let out = reader_cmd(bin, tmp.path()).arg(&path).args(["--turns", "4"]).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{bin}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("turn 4 is beyond the last turn (3)"), "{bin}: {}", String::from_utf8_lossy(&out.stderr));
        // a regex that does not compile is literal text, and stderr says so
        let out = reader_cmd(bin, tmp.path()).arg(&path).args(["--regex", "--grep", "wombats ("]).output().unwrap();
        assert!(out.status.success(), "{bin}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("term 1 is not a valid regular expression, matched as literal text"));
    }
}

#[test]
fn malformed_cache_fields_do_not_remove_records_from_either_export() {
    for (field, value) in [("effort", json!({"level":"high"})), ("requestRef", json!(123))] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tolerant.jsonl");
        let mut tool = json!({"type":"assistant","message":{"id":"tool-msg","stop_reason":"tool_use","content":[
            {"type":"tool_use","id":"tool","name":"Read","input":{"file_path":"/example"}}
        ]}});
        let mut reply = json!({"type":"assistant","message":{"id":"reply-msg","stop_reason":"end_turn","content":[
            {"type":"text","text":"preserved final answer"}
        ]}});
        tool[field] = value.clone(); reply[field] = value;
        let rows = [json!({"type":"user","message":{"content":"go"}}), tool, reply];
        std::fs::write(&path, rows.iter().map(|r| format!("{r}\n")).collect::<String>()).unwrap();
        let full = read_transcript(&path).unwrap();
        assert_eq!(full.skipped_lines, 0, "{field}");
        assert!(full.text.contains("preserved final answer"), "{field}");
        assert!(full.text.contains("Called Read 1 time."), "{field}");
        let selected = read_transcript_with(&path, &TranscriptOptions { turns: range("1"), tools: true, ..TranscriptOptions::default() }).unwrap();
        assert_eq!(selected.skipped_lines, 0, "{field}");
        assert!(selected.text.contains("preserved final answer"), "{field}");
        assert!(selected.text.contains("/example"), "{field}");
    }
}
