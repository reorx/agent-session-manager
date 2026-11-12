use std::path::Path;
use agent_session_core::claude::conversation::{read_conversation, tool_arg, tool_input_lines, ENTRY_CHARS, RESULT_CHARS};
use agent_session_core::model::*;
use agent_session_core::search::{Matcher, regex::ToolLineKind};
use serde_json::{json, Value};

fn fixture(matcher: Option<&Matcher>) -> Conversation {
    read_conversation(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/conversation.jsonl")), matcher).unwrap()
}
fn turn(c: &Conversation, i: usize) -> &Turn {
    let ConvItem::Turn(t) = &c.items[i] else { panic!("expected turn") }; t
}
fn tool(c: &Conversation, i: usize) -> &ToolCall {
    let StepKind::Tool(t) = &turn(c, 1).steps[i].kind else { panic!("expected tool") }; t
}
fn read(rows: Vec<Value>) -> Conversation {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.jsonl");
    let text = rows.iter().map(|r| format!("{r}\n")).collect::<String>();
    std::fs::write(&path, text).unwrap();
    read_conversation(&path, None).unwrap()
}
fn u(s: &str) -> Value { json!({"type":"user","message":{"content":s}}) }
fn a(s: &str) -> Value { json!({"type":"assistant","message":{"content":[{"type":"text","text":s}]}}) }

#[test]
fn turns_start_at_human_inputs_and_are_numbered_from_one() {
    let c = fixture(None);
    assert_eq!(turn(&c, 1).number, Some(1)); assert_eq!(turn(&c, 2).number, Some(2));
    assert_eq!(c.items.len(), 5);
}
#[test]
fn steps_keep_file_order_and_the_last_text_only_message_is_the_reply() {
    let c = fixture(None); let t = turn(&c, 1);
    assert_eq!(t.steps.len(), 5);
    assert!(matches!(t.steps[0].kind, StepKind::Thinking(_)));
    assert!(matches!(&t.steps[1].kind, StepKind::Text(m) if m.text == "working"));
    assert!(matches!(t.end, TurnEnd::Completed));
}
#[test]
fn a_reply_written_as_two_records_of_one_message_is_joined() {
    assert_eq!(turn(&fixture(None), 1).reply.as_ref().unwrap().text.text, "reply needle\n\nsecond block");
}
#[test]
fn a_tool_result_is_attached_to_the_call_it_answers() {
    let c = fixture(None);
    assert!(tool(&c, 2).result.as_ref().unwrap().output.text.starts_with("line\n"));
    assert_eq!(tool(&c, 3).result.as_ref().unwrap().output.text, "array result");
}
#[test]
fn a_tool_result_without_its_call_becomes_a_step_of_its_own() {
    let c = fixture(None); assert_eq!(tool(&c, 4).name, "?");
    assert_eq!(tool(&c, 4).result.as_ref().unwrap().output.text, "orphan");
}
#[test]
fn a_failed_tool_call_is_marked() { assert!(tool(&fixture(None), 2).result.as_ref().unwrap().is_error); }
#[test]
fn an_interruption_ends_the_turn_without_starting_a_new_one() {
    let c = fixture(None); assert!(matches!(turn(&c, 2).end, TurnEnd::Interrupted { .. }));
    assert!(turn(&c, 2).reply.is_none());
}
#[test]
fn an_api_error_ends_the_turn_and_a_later_reply_clears_it() {
    assert!(matches!(turn(&fixture(None), 3).end, TurnEnd::ApiError { .. }));
    let err = json!({"type":"assistant","isApiErrorMessage":true,"message":{"content":"API Error: unavailable"}});
    let c = read(vec![u("hi"), err, a("recovered")]);
    assert!(matches!(turn(&c, 0).end, TurnEnd::Completed));
    assert_eq!(turn(&c, 0).reply.as_ref().unwrap().text.text, "recovered");
}
#[test]
fn a_turn_without_a_reply_ends_as_no_reply() { assert!(matches!(turn(&read(vec![u("pending")]), 0).end, TurnEnd::NoReply)); }
#[test]
fn notifications_form_unnumbered_auto_segments_with_their_origin() {
    let c = fixture(None); assert_eq!(turn(&c, 3).number, None);
    assert_eq!(turn(&c, 3).origin.as_deref(), Some("task-notification"));
}
#[test]
fn activity_before_the_first_input_forms_a_session_start_segment() {
    let c = fixture(None); assert_eq!(turn(&c, 0).origin.as_deref(), Some("session start"));
}
#[test]
fn a_compaction_is_a_divider_carrying_its_summary() {
    let c = fixture(None);
    assert!(matches!(&c.items[4], ConvItem::Compact(c) if c.summary.as_ref().unwrap().text == "summary needle"));
    let c = read(vec![json!({"type":"user","isCompactSummary":true,"message":{"content":"old summary"}})]);
    assert!(matches!(&c.items[0], ConvItem::Compact(_)));
}
#[test]
fn images_in_a_prompt_become_attachments_with_their_size() {
    let c = fixture(None); let t = turn(&c, 1);
    assert_eq!(t.prompt.text, "first needle"); assert_eq!(t.attachments[0].kind, "image"); assert_eq!(t.attachments[0].bytes, 3);
}
#[test]
fn injected_messages_duplicates_and_empty_thinking_are_left_out() {
    let c = fixture(None); assert_eq!(turn(&c, 1).steps.len(), 5);
    assert!(!format!("{c:?}").contains("injected")); assert!(!format!("{c:?}").contains("duplicate"));
}
#[test]
fn turn_durations_add_up() { assert_eq!(turn(&fixture(None), 1).duration_ms, Some(300)); }
#[test]
fn a_half_written_last_line_is_ignored() { assert_eq!(fixture(None).skipped_lines, 1); }
#[test]
fn a_long_tool_result_is_cut_and_keeps_its_real_size_and_line_count() {
    let c = fixture(None); let r = tool(&c, 2).result.as_ref().unwrap();
    assert_eq!(r.bytes, 4506); assert_eq!(r.lines, 901); assert_eq!(r.output.chars, 4506);
    assert!(r.output.text.chars().count() <= RESULT_CHARS + 1);
    assert_eq!(r.output.kept, RESULT_CHARS, "kept: the characters of the original that the text holds");
}
#[test]
fn a_marked_text_counts_the_characters_it_keeps() {
    let whole = Marked::new("short", RESULT_CHARS, None);
    assert_eq!((whole.kept, whole.chars), (5, 5), "nothing cut: everything kept");
    let cut = Marked::new(&"é".repeat(5000), RESULT_CHARS, None);
    assert_eq!((cut.kept, cut.chars), (RESULT_CHARS, 5000), "the trailing ellipsis is not counted");
    let near_hit = format!("{}needle{}", "a".repeat(5000), "b".repeat(10));
    let moved = Marked::new(&near_hit, RESULT_CHARS, Some(&Matcher::regex("needle", true)));
    assert!(moved.moved && moved.text.starts_with('…') && !moved.text.ends_with('…'));
    assert_eq!(moved.kept, 200 + 6 + 10, "a window moved to a hit keeps from the lead to the end");
}
#[test]
fn tool_input_lists_description_command_then_the_other_fields() {
    let v = json!({"timeout":1000,"command":"echo needle","description":"inspect"});
    let lines = tool_input_lines(v.as_object().unwrap(), Some(&Matcher::regex("needle", true)));
    assert_eq!(lines[0].kind, ToolLineKind::Description); assert_eq!(lines[1].kind, ToolLineKind::Command);
    assert_eq!(lines[2].kind, ToolLineKind::Field("timeout".into())); assert!(!lines[1].text.ranges.is_empty());
}
#[test]
fn the_tool_argument_is_the_most_telling_field() {
    for (name, value, expected) in [
        ("Bash", json!({"command":"echo hi\nexit"}), "echo hi"),
        ("Read", json!({"file_path":"/tmp/a"}), "/tmp/a"),
        ("Grep", json!({"pattern":"needle","path":"src"}), "needle src"),
        ("WebFetch", json!({"url":"https://example.com"}), "https://example.com"),
        ("Task", json!({"description":"inspect"}), "inspect"),
        ("Unknown", json!({"n":1,"value":"fallback"}), "fallback"),
    ] { assert_eq!(tool_arg(name, value.as_object().unwrap()), expected); }
}
#[test]
fn search_hits_are_marked_in_prompts_steps_replies_and_summaries() {
    let c = fixture(Some(&Matcher::regex("needle", true))); let t = turn(&c, 1);
    assert!(!t.prompt.hits.is_empty()); assert!(!t.reply.as_ref().unwrap().text.hits.is_empty());
    assert!(matches!(&t.steps[0].kind, StepKind::Thinking(m) if !m.hits.is_empty()));
    assert!(tool(&c, 2).result.as_ref().unwrap().output.moved);
}
#[test]
fn a_hit_past_the_cut_moves_the_cut() {
    let text = format!("{}needle末尾", "中".repeat(ENTRY_CHARS + 100));
    let m = Marked::new(&text, ENTRY_CHARS, Some(&Matcher::regex("needle", true)));
    assert!(m.moved); assert!(m.text.starts_with('…')); assert_eq!(&m.text[m.hits[0].clone()], "needle");
    assert_eq!(m.chars, ENTRY_CHARS + 108);
}
#[test]
fn hit_places_list_every_part_with_a_hit_in_file_order() {
    let c = fixture(Some(&Matcher::regex("needle", true)));
    let places: Vec<_> = c.hit_places().into_iter().map(|h| (h.item, h.part)).collect();
    assert_eq!(places, vec![(1, Part::Prompt), (1, Part::Step(0)), (1, Part::Step(2)), (1, Part::Reply), (3, Part::Prompt), (4, Part::Summary)]);
}
#[test]
fn a_subagent_transcript_reads_like_a_session() {
    let dir = tempfile::tempdir().unwrap(); let sub = dir.path().join("subagents"); std::fs::create_dir(&sub).unwrap();
    let path = sub.join("agent.jsonl");
    let text = "{\"type\":\"user\",\"isSidechain\":true,\"message\":{\"content\":\"hello\"}}\n";
    std::fs::write(&path, text).unwrap(); assert_eq!(turn(&read_conversation(&path, None).unwrap(), 0).number, Some(1));
    let parent = dir.path().join("parent.jsonl"); std::fs::write(&parent, text).unwrap();
    assert!(read_conversation(&parent, None).unwrap().items.is_empty());
}

#[test]
fn text_blocks_in_one_legacy_message_without_an_id_form_one_reply() {
    let c = read(vec![u("hi"), json!({"type":"assistant","message":{"content":[
        {"type":"text","text":"first"},{"type":"text","text":"second"}
    ]}})]);
    assert_eq!(turn(&c, 0).reply.as_ref().unwrap().text.text, "first\n\nsecond");
    assert!(turn(&c, 0).steps.is_empty());
}

#[test]
fn tools_in_another_record_of_the_same_message_make_all_its_text_commentary() {
    let c = read(vec![json!({"type":"assistant","message":{"id":"same","content":[{"type":"tool_use","name":"Bash","id":"b"}]}}),
        json!({"type":"assistant","message":{"id":"same","content":[{"type":"text","text":"commentary"}]}})]);
    assert!(turn(&c, 0).reply.is_none());
    assert_eq!(turn(&c, 0).steps.len(), 2);
}

#[test]
fn cli_reads_without_creating_an_index_and_requires_conversation_for_match() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("unused.sqlite");
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/conversation.jsonl");
    assert_cmd::Command::cargo_bin("asm").unwrap().args(["--db", db.to_str().unwrap(), "session", fixture, "--conversation", "--match", "needle"])
        .assert().success().stdout(predicates::str::contains("total turns=2 auto=2 compact=1 tools=3 completed=2 interrupted=1 api-error=1 no-reply=0 hits=6 skipped=1"));
    assert!(!db.exists());
    assert_cmd::Command::cargo_bin("asm").unwrap().args(["session", fixture, "--match", "needle"]).assert().failure();
}

#[test]
fn matcher_patterns_distinguish_case_sensitive_searches() {
    assert_ne!(Matcher::regex("needle", true).pattern(), Matcher::regex("needle", false).pattern());
}

#[test]
fn late_tool_results_attach_across_turn_boundaries_and_keep_timestamps() {
    let c = read(vec![u("first"), json!({"type":"assistant","timestamp":"2026-10-26T00:00:01Z","message":{"content":[{"type":"tool_use","name":"Read","id":"call"}]}}),
        u("second"), json!({"type":"user","timestamp":"2026-10-26T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"call","content":"late"}]}})]);
    let StepKind::Tool(call) = &turn(&c, 0).steps[0].kind else { panic!() };
    assert_eq!(call.result.as_ref().unwrap().output.text, "late");
    assert_eq!(turn(&c, 0).last_ts.unwrap() - turn(&c, 0).steps[0].ts.unwrap(), 2000);
    assert!(turn(&c, 1).steps.is_empty());
}

#[test]
fn all_turns_are_read_without_a_message_limit() {
    let c = read((0..350).flat_map(|i| [u(&format!("question {i}")), a("answer")]).collect());
    assert_eq!(c.items.len(), 350); assert_eq!(turn(&c, 349).number, Some(350));
    assert_eq!(turn(&c, 349).reply.as_ref().unwrap().text.text, "answer");
}

#[test]
fn an_aborted_assistant_and_later_thinking_follow_the_last_activity() {
    let aborted = json!({"type":"assistant","isAbortedMidStream":true,"message":{"content":"unfinished"}});
    let c = read(vec![u("hi"), aborted.clone()]);
    assert!(matches!(turn(&c, 0).end, TurnEnd::Interrupted { .. })); assert!(turn(&c, 0).reply.is_none());
    let c = read(vec![u("hi"), aborted, json!({"type":"assistant","message":{"content":[{"type":"thinking","thinking":"retry"}]}})]);
    assert!(matches!(turn(&c, 0).end, TurnEnd::NoReply));
}

#[test]
fn long_input_fields_move_to_matches_and_keep_unicode_boundaries() {
    let value = format!("{}needle", "中".repeat(2500));
    let input = json!({"command":value});
    let lines = tool_input_lines(input.as_object().unwrap(), Some(&Matcher::regex("needle", true)));
    assert!(lines[0].text.text.starts_with('…'));
    assert_eq!(&lines[0].text.text[lines[0].text.ranges[0].clone()], "needle");
}

#[test]
fn an_explicit_automatic_origin_is_kept_even_when_the_harness_marks_it_meta() {
    let c = read(vec![json!({"type":"user","isMeta":true,"origin":{"kind":"peer"},"message":{"content":"peer update"}})]);
    assert_eq!(turn(&c, 0).origin.as_deref(), Some("peer"));
    assert_eq!(turn(&c, 0).prompt.text, "peer update");
}

#[test]
fn a_tool_use_stop_reason_prevents_a_prior_text_from_becoming_the_reply() {
    let c = read(vec![u("hi"), json!({"type":"assistant","message":{"id":"m","content":"working"}}),
        json!({"type":"assistant","message":{"id":"m","stop_reason":"tool_use","content":[]}})]);
    assert!(turn(&c, 0).reply.is_none());
    assert!(matches!(turn(&c, 0).end, TurnEnd::NoReply));
}

#[test]
fn a_history_conversation_is_one_prompt_only_turn_per_prompt_with_hits_marked() {
    use agent_session_core::claude::conversation::conversation_from_history;
    use agent_session_core::model::HistoryPrompt;
    let prompts = vec![
        HistoryPrompt { ts: 123, text: "中文 needle".into() },
        HistoryPrompt { ts: 456, text: format!("{}needle", "长".repeat(ENTRY_CHARS + 1)) },
        HistoryPrompt { ts: 789, text: String::new() },
    ];
    let matcher = Matcher::regex("needle", true);
    let c = conversation_from_history(&prompts, Some(&matcher));
    assert!(c.history_only);
    assert!(!fixture(None).history_only);
    assert_eq!(c.items.len(), prompts.len());
    assert_eq!((c.bytes, c.skipped_lines), (0, 0));
    for (i, p) in prompts.iter().enumerate() {
        let t = turn(&c, i);
        assert_eq!(t.number, Some(i as u32 + 1));
        assert_eq!(t.ts, Some(p.ts));
        assert_eq!(t.last_ts, Some(p.ts));
        assert!(t.origin.is_none() && t.duration_ms.is_none());
        assert!(t.attachments.is_empty() && t.steps.is_empty() && t.reply.is_none());
        assert!(matches!(t.end, TurnEnd::NoReply));
    }
    assert_eq!(turn(&c, 0).prompt.hits, vec![7..13]);
    assert!(turn(&c, 1).prompt.moved);
    assert_eq!(c.hit_places(), vec![HitPlace { item: 0, part: Part::Prompt }, HitPlace { item: 1, part: Part::Prompt }]);
    let empty = conversation_from_history(&[], None);
    assert!(empty.history_only && empty.items.is_empty());
    assert!(conversation_from_history(&prompts, None).hit_places().is_empty());
}

// Cache breaks: a main-thread call that reads less than the call before it sent.

const OPUS: &str = "claude-opus-5-5";

/// `secs` after 10:00:00.
fn at(secs: u32) -> String {
    let s = 10 * 3600 + secs;
    format!("2026-11-12T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}
fn prompt_at(text: &str, secs: u32) -> Value { json!({"type":"user","timestamp":at(secs),"message":{"content":text}}) }
/// One API response with 3 uncached input tokens: `read` from the cache, `write` to it (1-hour
/// writes when `hour`).
fn call(id: &str, secs: u32, model: &str, read: u64, write: u64, hour: bool) -> Value {
    let (w5, w1) = if hour { (0, write) } else { (write, 0) };
    json!({"type":"assistant","timestamp":at(secs),"message":{"id":id,"model":model,"content":[{"type":"text","text":"ok"}],
        "usage":{"input_tokens":3,"cache_creation_input_tokens":write,"cache_read_input_tokens":read,"output_tokens":10,
            "cache_creation":{"ephemeral_5m_input_tokens":w5,"ephemeral_1h_input_tokens":w1}}}})
}
/// (item index, break) of every break in the conversation.
fn breaks(c: &Conversation) -> Vec<(usize, &CacheBreak)> {
    c.items.iter().enumerate().flat_map(|(i, item)| match item {
        ConvItem::Turn(t) => t.cache_breaks.iter().map(|b| (i, b)).collect(),
        ConvItem::Compact(_) => Vec::new(),
    }).collect()
}

#[test]
fn calls_that_reuse_the_cache_mark_nothing_and_the_first_call_never_does() {
    let c = read(vec![
        prompt_at("go", 0), call("m1", 1, OPUS, 0, 50_000, true), call("m2", 2, OPUS, 50_003, 2_000, true),
        prompt_at("more", 600), call("m3", 601, OPUS, 52_006, 1_000, true),
    ]);
    assert!(breaks(&c).is_empty());
}

#[test]
fn a_call_after_the_cache_expired_is_a_break_on_its_turn_with_the_idle_time() {
    let c = read(vec![
        prompt_at("go", 0), call("m1", 10, OPUS, 0, 80_000, true),
        prompt_at("back", 7_800), call("m2", 7_810, OPUS, 20_000, 62_000, true),
    ]);
    let found = breaks(&c);
    assert_eq!(found.len(), 1);
    let (item, b) = found[0];
    assert_eq!(item, 1, "on the turn the call belongs to");
    assert_eq!((b.dropped, b.written), (80_003 - 20_000, 62_000));
    assert_eq!(b.cause, CacheBreakCause::Idle { gap_ms: 7_800_000, ttl_ms: 3_600_000 });
    assert!((b.cost_usd.unwrap() - 0.496).abs() < 1e-9, "62k 1-hour writes at $8 per million");
    assert_eq!(b.ts, turn(&c, 1).ts.map(|t| t + 10_000));
}

#[test]
fn five_minute_writes_expire_after_five_minutes() {
    let c = read(vec![
        prompt_at("go", 0), call("m1", 1, OPUS, 0, 30_000, false),
        prompt_at("back", 600), call("m2", 601, OPUS, 0, 31_000, false),
    ]);
    let b = breaks(&c)[0].1;
    assert_eq!(b.cause, CacheBreakCause::Idle { gap_ms: 600_000, ttl_ms: 300_000 });
    assert!((b.cost_usd.unwrap() - 0.155).abs() < 1e-9, "31k 5-minute writes at $5 per million");
}

#[test]
fn a_compaction_a_model_switch_or_nothing_visible_can_break_the_cache() {
    let compacted = read(vec![
        prompt_at("go", 0), call("m1", 1, OPUS, 0, 100_000, true),
        json!({"type":"system","subtype":"compact_boundary","timestamp":at(30)}),
        json!({"type":"user","isCompactSummary":true,"timestamp":at(31),"message":{"content":"summary"}}),
        call("m2", 40, OPUS, 20_000, 15_000, true),
    ]);
    assert_eq!(breaks(&compacted)[0].1.cause, CacheBreakCause::Compaction);
    let switched = read(vec![prompt_at("go", 0), call("m1", 1, "claude-opus-4-7", 0, 40_000, true), prompt_at("go on", 60), call("m2", 61, OPUS, 0, 41_000, true)]);
    assert_eq!(breaks(&switched)[0].1.cause, CacheBreakCause::ModelSwitch { from: "claude-opus-4-7".into() });
    let unknown = read(vec![prompt_at("go", 0), call("m1", 1, OPUS, 29_000, 60_000, true), call("m2", 4, OPUS, 29_000, 61_000, true)]);
    assert_eq!(breaks(&unknown)[0].1.cause, CacheBreakCause::Unknown);
}

#[test]
fn a_response_written_as_two_records_counts_once_and_small_drops_do_not_count() {
    let split = call("m2", 7_810, OPUS, 20_000, 62_000, true);
    let c = read(vec![prompt_at("go", 0), call("m1", 10, OPUS, 0, 80_000, true), prompt_at("back", 7_800), split.clone(), split]);
    assert_eq!(breaks(&c).len(), 1, "the second record repeats the usage of the same response");
    let small = read(vec![prompt_at("go", 0), call("m1", 1, OPUS, 0, 50_000, true), prompt_at("x", 900), call("m2", 901, OPUS, 41_000, 9_500, true)]);
    assert!(breaks(&small).is_empty(), "9,003 tokens dropped is under the 10k bar");
}

#[test]
fn records_claude_code_wrote_itself_are_not_calls() {
    let synthetic = json!({"type":"assistant","timestamp":at(20),"message":{"id":"x","model":"<synthetic>","content":[{"type":"text","text":"No response requested."}],
        "usage":{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}}});
    let c = read(vec![prompt_at("go", 0), call("m1", 1, OPUS, 0, 50_000, true), synthetic, call("m2", 30, OPUS, 50_003, 100, true)]);
    assert!(breaks(&c).is_empty());
}
