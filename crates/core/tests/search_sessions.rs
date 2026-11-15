//! Behavior: session-level search (`asm search`). TERMs are concepts a session must all contain,
//! `|` separates ways of saying one; the index answers prompt / reply / thinking / other, a scan of
//! the transcripts answers tool kinds and regexes, and both give the same results.

mod common;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use agent_session_core::claude::paths::ClaudeDir;
use agent_session_core::index::{Db, Indexer};
use agent_session_core::model::SessionSummary;
use agent_session_core::search::query::{Query, QueryError};
use agent_session_core::search::sessions::{
    Engine, Kind, KindError, KindSet, ProjectMatcher, SearchOptions, SessionFilter, SessionHits, SortBy, search,
    select_sessions, turn_count,
};
use serde_json::{Value, json};
use tempfile::TempDir;

// ---------------------------------------------------------------------------------------------
// Building transcripts

const BASE_MS: i64 = 1_793_527_200_000; // 2026-11-01T10:00:00Z

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// One transcript, written record by record; every record is one minute after the previous one.
struct T {
    sid: String,
    cwd: String,
    lines: Vec<String>,
    start_ms: i64,
    n: i64,
}

impl T {
    fn new(sid: &str, cwd: &str) -> T {
        T { sid: sid.into(), cwd: cwd.into(), lines: Vec::new(), start_ms: BASE_MS, n: 0 }
    }

    fn starting(mut self, ms: i64) -> T {
        self.start_ms = ms;
        self
    }

    fn next_ts(&mut self) -> (String, String) {
        self.n += 1;
        (format!("{}-{:04}", &self.sid[..8], self.n), iso(self.start_ms + self.n * 60_000))
    }

    fn push(&mut self, rec: Value) -> &mut T {
        self.lines.push(serde_json::to_string(&rec).unwrap());
        self
    }

    fn user_rec(&mut self, content: Value, extra: Value) -> &mut T {
        let (uuid, ts) = self.next_ts();
        let mut rec = json!({
            "type": "user", "uuid": uuid, "timestamp": ts, "isSidechain": false, "cwd": self.cwd,
            "sessionId": self.sid, "message": {"role": "user", "content": content}
        });
        for (k, v) in extra.as_object().unwrap() {
            rec[k] = v.clone();
        }
        self.push(rec)
    }

    fn prompt(&mut self, text: &str) -> &mut T {
        self.user_rec(json!(text), json!({"origin": {"kind": "human"}}))
    }

    fn assistant(&mut self, blocks: Value) -> &mut T {
        let (uuid, ts) = self.next_ts();
        let rec = json!({
            "type": "assistant", "uuid": uuid, "timestamp": ts, "isSidechain": false, "cwd": self.cwd,
            "sessionId": self.sid, "requestId": format!("req_{uuid}"),
            "message": {"model": "claude-opus-5-5", "id": format!("msg_{uuid}"), "type": "message", "role": "assistant",
                "content": blocks, "stop_reason": "end_turn",
                "usage": {"input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}
        });
        self.push(rec)
    }

    fn reply(&mut self, text: &str) -> &mut T {
        self.assistant(json!([{"type": "text", "text": text}]))
    }

    fn thinking_and_reply(&mut self, thinking: &str, text: &str) -> &mut T {
        self.assistant(json!([{"type": "thinking", "thinking": thinking, "signature": "sig"}, {"type": "text", "text": text}]))
    }

    /// A tool call and its result.
    fn tool(&mut self, id: &str, name: &str, input: Value, output: &str) -> &mut T {
        self.assistant(json!([{"type": "tool_use", "id": id, "name": name, "input": input}]));
        self.user_rec(json!([{"tool_use_id": id, "type": "tool_result", "content": output}]), json!({"toolUseResult": {"stdout": output}}))
    }

    fn notification(&mut self, text: &str) -> &mut T {
        self.user_rec(json!(text), json!({"origin": {"kind": "task-notification"}}))
    }

    fn meta(&mut self, text: &str) -> &mut T {
        self.user_rec(json!([{"type": "text", "text": text}]), json!({"isMeta": true}))
    }

    fn interrupt(&mut self) -> &mut T {
        self.user_rec(json!("[Request interrupted by user]"), json!({"origin": {"kind": "human"}}))
    }

    fn text(&self) -> String {
        self.lines.iter().map(|l| format!("{l}\n")).collect()
    }
}

/// A Claude data directory with the given transcripts, indexed.
struct World {
    tmp: TempDir,
    db: Db,
}

impl World {
    fn claude_root(&self) -> PathBuf {
        self.tmp.path().join("claude")
    }
}

fn key_of(cwd: &str) -> String {
    agent_session_core::claude::paths::encode_project_path(cwd)
}

/// `(transcript, mtime)`: the transcript lands in the project directory of its cwd.
fn world(transcripts: &[(&T, i64)]) -> World {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("claude");
    for (t, mtime) in transcripts {
        let dir = root.join("projects").join(key_of(&t.cwd));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.jsonl", t.sid));
        fs::write(&path, t.text()).unwrap();
        common::set_mtime(&path, *mtime);
    }
    fs::create_dir_all(root.join("projects")).unwrap();
    let mut db = Db::open(&tmp.path().join("index.sqlite")).unwrap();
    Indexer::new(ClaudeDir::new(&root), None).scan(&mut db, &mut |_| {}).unwrap();
    World { tmp, db }
}

fn terms(t: &[&str]) -> Vec<String> {
    t.iter().map(|s| s.to_string()).collect()
}

fn query(t: &[&str]) -> Query {
    Query::parse(&terms(t), false, false).unwrap()
}

fn kinds(in_: Option<&str>, tools: &[&str]) -> KindSet {
    KindSet::parse(in_, &terms(tools)).unwrap()
}

fn all_sessions(w: &World) -> Vec<SessionSummary> {
    select_sessions(&w.db, &SessionFilter::default()).unwrap()
}

fn opts(engine: Option<Engine>, snippets: usize) -> SearchOptions {
    SearchOptions { snippets, engine, ..SearchOptions::default() }
}

/// Search with the engine the query calls for. When the index can answer, the scan must give
/// exactly the same results (and so must a scan without its byte prefilter).
fn run(w: &World, q: &Query, k: &KindSet, snippets: usize) -> Vec<SessionHits> {
    run_on(w, &all_sessions(w), q, k, snippets)
}

fn run_on(w: &World, sessions: &[SessionSummary], q: &Query, k: &KindSet, snippets: usize) -> Vec<SessionHits> {
    let out = search(&w.db, sessions, q, k, &opts(None, snippets)).unwrap();
    let scan = search(&w.db, sessions, q, k, &opts(Some(Engine::Scan), snippets)).unwrap();
    let raw = search(&w.db, sessions, q, k, &SearchOptions { prefilter: false, ..opts(Some(Engine::Scan), snippets) }).unwrap();
    if out.engine == Engine::Index {
        assert_eq!(out.sessions, scan.sessions, "index and scan disagree");
    }
    assert_eq!(scan.sessions, raw.sessions, "the prefilter changed the scan's results");
    out.sessions
}

fn ids(hits: &[SessionHits]) -> Vec<&str> {
    hits.iter().map(|h| h.session_id.as_str()).collect()
}

fn count(h: &SessionHits, kind: Kind) -> Option<u32> {
    h.counts.get(&kind).copied()
}

const SA: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const SB: &str = "bbbbbbbb-0000-4000-8000-00000000000b";
const SC: &str = "cccccccc-0000-4000-8000-00000000000c";

const DAY: i64 = 86_400_000;

// ---------------------------------------------------------------------------------------------
// TERMs

#[test]
fn bars_separate_alternatives_whose_surrounding_spaces_and_empty_ones_are_dropped() {
    let q = Query::parse(&terms(&["连不上 | 无法连接 |", " offline|unreachable ", "a b"]), false, false).unwrap();
    let alts: Vec<Vec<&str>> = q.terms().iter().map(|t| t.alternatives.iter().map(String::as_str).collect()).collect();
    assert_eq!(alts, vec![vec!["连不上", "无法连接"], vec!["offline", "unreachable"], vec!["a b"]]);
    assert_eq!(q.terms_in("现场无法连接了"), vec![0]);
    assert_eq!(q.terms_in("OFFLINE, a b"), vec![1, 2], "case-insensitive; spaces inside a TERM stay");
    assert!(q.terms_in("ab").is_empty());
}

#[test]
fn a_term_with_nothing_to_match_is_an_error() {
    for bad in ["", "|", " | ", "  "] {
        assert_eq!(Query::parse(&terms(&["ok", bad]), false, false).err(), Some(QueryError::EmptyTerm(1)), "{bad:?}");
    }
    for bad in ["", "a|", "x*", "foo|\\b", "\\B", "(?:)", "^"] {
        assert_eq!(Query::parse(&terms(&[bad]), true, false).err(), Some(QueryError::MatchesEmpty(0)), "{bad:?}");
    }
    assert!(Query::parse(&[], false, false).is_err());
    let many: Vec<String> = (0..65).map(|i| format!("t{i}")).collect();
    assert_eq!(Query::parse(&many, false, false).err(), Some(QueryError::TooManyTerms));
    let big = ["\\w{200}", "x\\w{200}"];
    for one in big {
        assert_eq!(Query::parse(&terms(&[one]), true, false).unwrap().regex_fallback(), Vec::<usize>::new(), "{one} compiles alone");
    }
    assert_eq!(Query::parse(&terms(&big), true, false).err(), Some(QueryError::TooLarge), "but not with the other, to highlight both");
    assert!(QueryError::EmptyTerm(1).to_string().contains("term 2"), "messages count TERMs from 1");
}

#[test]
fn literal_terms_fold_unicode_case() {
    let q = query(&["ÄRGER", "Straße"]);
    assert_eq!(q.terms_in("großer ärger in der STRASSE straße"), vec![0, 1]);
    assert_eq!(query(&["δοκιμή"]).terms_in("ΔΟΚΙΜΉ"), vec![0]);
}

#[test]
fn regex_terms_are_case_insensitive_unless_turned_off_and_bars_are_alternation() {
    let q = Query::parse(&terms(&["ERR\\d+", "(?-i)Fatal", "conn(ection)? (refused|reset)"]), true, false).unwrap();
    assert!(q.regex_fallback().is_empty());
    assert_eq!(q.terms_in("err42 Fatal connection reset"), vec![0, 1, 2]);
    assert_eq!(q.terms_in("err42 fatal conn refused"), vec![0, 2]);
    assert_eq!(q.terms()[2].alternatives, vec!["conn(ection)? (refused|reset)".to_string()], "the whole TERM is one entry");
}

#[test]
fn an_invalid_regex_is_matched_as_literal_text_without_splitting() {
    let q = Query::parse(&terms(&["ok", "foo(|bar"]), true, false).unwrap();
    assert_eq!(q.regex_fallback(), vec![1]);
    assert_eq!(q.terms_in("ok FOO(|BAR"), vec![0, 1]);
    assert_eq!(q.terms_in("ok foo bar"), vec![0]);
}

#[test]
fn the_matcher_highlights_every_term() {
    let q = query(&["healthz", "连不上|offline"]);
    let h = q.matcher().snippet("before 现场连不上, healthz is offline", 60, 140);
    assert_eq!(h.render("«", "»"), "before 现场«连不上», «healthz» is «offline»");
}

// ---------------------------------------------------------------------------------------------
// Text kinds (`--in`, `--tool`)

#[test]
fn kinds_default_to_conversation_text_and_tool_alone_means_its_calls() {
    assert_eq!(kinds(None, &[]).kinds(), vec![Kind::Prompt, Kind::Reply, Kind::Thinking]);
    let k = kinds(None, &["Bash"]);
    assert_eq!(k.kinds(), vec![Kind::ToolInput, Kind::ToolOutput]);
    assert!(k.tool_matches("bash") && !k.tool_matches("Read"));
    assert_eq!(kinds(Some("other,prompt,tool-output"), &[]).kinds(), vec![Kind::Prompt, Kind::ToolOutput, Kind::Other], "table order");
    assert_eq!(kinds(Some("all"), &[]).kinds(), Kind::ALL.to_vec());
    assert_eq!(kinds(Some("prompt,tool"), &["Read"]).kinds(), vec![Kind::Prompt, Kind::ToolInput, Kind::ToolOutput]);
    assert!(kinds(Some("prompt,tool"), &[]).tool_matches("anything"));
    assert!(kinds(Some("prompt, reply ,thinking,other"), &[]).index_only());
    assert!(!kinds(Some("prompt,tool-input"), &[]).index_only());
    assert_eq!(KindSet::parse(Some("prompt,tools"), &[]).err(), Some(KindError::Unknown("tools".into())));
    assert_eq!(KindSet::parse(Some("prompt,reply"), &terms(&["Bash"])).err(), Some(KindError::ToolWithoutToolKind));
    assert_eq!(KindSet::parse(Some(" , "), &[]).err(), Some(KindError::Empty));
    assert_eq!(Kind::ToolOutput.as_str(), "tool-output");
    assert_eq!(Kind::parse("tool-input"), Some(Kind::ToolInput));
}

// ---------------------------------------------------------------------------------------------
// Session-level matching

fn concepts_world() -> World {
    let mut a = T::new(SA, "/Users/me/Code/edge");
    a.prompt("为什么 healthz 报错").reply("现场连不上了").prompt("继续").reply("好的");
    let mut b = T::new(SB, "/Users/me/Code/edge");
    b.prompt("看一下").reply("healthz 在 app offline 时返回 503");
    let mut c = T::new(SC, "/Users/me/Code/vibench");
    c.prompt("maiev 一直无法连接").reply("先看日志");
    world(&[(&a, BASE_MS + DAY), (&b, BASE_MS + 2 * DAY), (&c, BASE_MS + 3 * DAY)])
}

#[test]
fn every_concept_must_occur_but_they_may_sit_in_different_items() {
    let w = concepts_world();
    let k = kinds(None, &[]);
    let hits = run(&w, &query(&["连不上|无法连接|offline", "healthz"]), &k, 3);
    assert_eq!(ids(&hits), vec![SB, SA], "most recently active first; SC lacks healthz");
    let same = run(&w, &Query::parse(&terms(&["连不上|无法连接|offline", "healthz"]), false, true).unwrap(), &k, 3);
    assert_eq!(ids(&same), vec![SB], "only SB has both in one item");
    let any = run(&w, &query(&["连不上|无法连接|offline"]), &k, 3);
    assert_eq!(ids(&any), vec![SC, SB, SA], "any alternative counts");
}

#[test]
fn hits_count_items_per_kind_and_per_term() {
    let w = concepts_world();
    let hits = run(&w, &query(&["连不上|offline", "healthz"]), &kinds(None, &[]), 3);
    let a = hits.iter().find(|h| h.session_id == SA).unwrap();
    assert_eq!((count(a, Kind::Prompt), count(a, Kind::Reply), count(a, Kind::Thinking)), (Some(1), Some(1), Some(0)));
    assert_eq!(count(a, Kind::Other), None, "kinds not searched have no count");
    assert_eq!(a.total(), 2);
    assert_eq!(a.term_hits, vec![1, 1]);
    let b = hits.iter().find(|h| h.session_id == SB).unwrap();
    assert_eq!((b.total(), b.term_hits.clone()), (1, vec![1, 1]), "one item with both TERMs is one hit");
}

#[test]
fn snippets_show_kind_turn_time_offset_and_marked_text_best_items_first() {
    let w = concepts_world();
    let hits = run(&w, &query(&["连不上|offline", "healthz"]), &kinds(None, &[]), 3);
    let a = hits.iter().find(|h| h.session_id == SA).unwrap();
    let shown: Vec<(Kind, Option<u32>, Vec<usize>, String)> =
        a.snippets.iter().map(|s| (s.kind, s.turn, s.terms.clone(), s.text.render("«", "»"))).collect();
    assert_eq!(shown, vec![
        (Kind::Prompt, Some(1), vec![1], "为什么 «healthz» 报错".to_string()),
        (Kind::Reply, Some(1), vec![0], "现场«连不上»了".to_string()),
    ]);
    let first = &a.snippets[0];
    assert_eq!(first.ts, Some(BASE_MS + 60_000));
    let file = fs::read(w.claude_root().join("projects").join(key_of("/Users/me/Code/edge")).join(format!("{SA}.jsonl"))).unwrap();
    assert!(file[first.offset as usize..].starts_with(b"{\"type\":\"user\""), "offset of the item's JSONL line");
    assert_eq!(first.tool, None);
}

#[test]
fn snippet_limit_still_shows_every_term_first() {
    let mut a = T::new(SA, "/Users/me/Code/edge");
    a.prompt("alpha one").prompt("alpha two").prompt("alpha three").reply("beta here").notification("alpha beta together");
    let w = world(&[(&a, BASE_MS + DAY)]);
    let q = query(&["alpha", "beta"]);
    let hits = run(&w, &q, &kinds(Some("all"), &[]), 2);
    let s: Vec<(Kind, Vec<usize>)> = hits[0].snippets.iter().map(|s| (s.kind, s.terms.clone())).collect();
    assert_eq!(s, vec![(Kind::Other, vec![0, 1]), (Kind::Prompt, vec![0])], "more TERMs first, then kind, then position");
    let hits = run(&w, &q, &kinds(Some("prompt,reply"), &[]), 2);
    let s: Vec<(Kind, Option<u32>)> = hits[0].snippets.iter().map(|s| (s.kind, s.turn)).collect();
    assert_eq!(s, vec![(Kind::Prompt, Some(1)), (Kind::Reply, Some(3))], "the reply is picked to show beta");
    assert!(run(&w, &q, &kinds(Some("all"), &[]), 0)[0].snippets.is_empty());
}

#[test]
fn kinds_choose_the_text_searched() {
    let mut a = T::new(SA, "/Users/me/Code/edge");
    a.prompt("first question")
        .thinking_and_reply("pondering zanzibar quietly", "visible answer")
        .notification("background task quokka finished")
        .meta("skill body mentions narwhal")
        .tool("toolu_1", "Bash", json!({"command": "ls /tmp/wombat", "description": "List the wombat dir"}), "wombat.txt\nexit code 7")
        .tool("toolu_2", "Read", json!({"file_path": "/x/quagga.conf", "limit": 20, "pages": [1, 2]}), "contents")
        .interrupt();
    let w = world(&[(&a, BASE_MS + DAY)]);
    let found = |term: &str, in_: Option<&str>, tools: &[&str]| !run(&w, &query(&[term]), &kinds(in_, tools), 3).is_empty();
    let found_re = |term: &str| !run(&w, &Query::parse(&terms(&[term]), true, false).unwrap(), &kinds(Some("tool-input"), &[]), 3).is_empty();
    assert!(found("zanzibar", None, &[]), "thinking is searched by default");
    assert!(!found("zanzibar", Some("prompt,reply"), &[]));
    assert!(!found("quokka", None, &[]), "other is not searched by default");
    assert!(found("quokka", Some("other"), &[]));
    assert!(found("request interrupted", Some("other"), &[]), "an interruption marker is other text");
    assert!(!found("request interrupted", Some("prompt"), &[]));
    assert!(!found("narwhal", Some("all"), &[]), "isMeta text is never searched");
    assert!(!found("exit code 7", None, &[]));
    assert!(found("exit code 7", Some("tool-output"), &[]));
    assert!(!found("exit code 7", Some("tool-input"), &[]));
    assert!(found("/tmp/wombat", Some("tool-input"), &[]));
    assert!(found("$ ls /tmp", Some("tool-input"), &[]), "a Bash command reads as `$ command`");
    assert!(found("# List the", Some("tool-input"), &[]), "its description as `# description`");
    assert!(found("file_path:", Some("tool-input"), &[]), "other fields as `key: value`");
    assert!(found("limit: 20", Some("tool-input"), &[]));
    assert!(found("pages: [1, 2]", Some("tool-input"), &[]), "nested values as JSON with spaces after `,` and `:`");
    assert!(found("$", Some("tool-input"), &[]));
    assert!(found("#", Some("tool-input"), &[]));
    assert!(found_re("file_path:.*quagga"));
    assert!(found_re(" /x/quagga"), "a regex keeps its spaces, and the text has one after `:`");
    assert!(found_re("\\[1, 2\\]"));
    assert!(found("wombat", None, &["bash"]), "--tool alone searches that tool's calls");
    assert!(!found("wombat", None, &["Read"]));
}

#[test]
fn tool_hits_name_their_tool_and_tool_filters_both_sides() {
    let mut a = T::new(SA, "/Users/me/Code/edge");
    a.prompt("run it")
        .tool("toolu_r", "Read", json!({"file_path": "/x/config.toml"}), "port = 8080")
        .tool("toolu_b", "Bash", json!({"command": "curl localhost:8080"}), "curl: (7) Failed to connect to localhost port 8080");
    let w = world(&[(&a, BASE_MS + DAY)]);
    let hits = run(&w, &query(&["8080"]), &kinds(Some("tool"), &[]), 5);
    let s: Vec<(Kind, Option<&str>)> = hits[0].snippets.iter().map(|s| (s.kind, s.tool.as_deref())).collect();
    assert_eq!(s, vec![(Kind::ToolInput, Some("Bash")), (Kind::ToolOutput, Some("Read")), (Kind::ToolOutput, Some("Bash"))]);
    assert_eq!((count(&hits[0], Kind::ToolInput), count(&hits[0], Kind::ToolOutput)), (Some(1), Some(2)));
    let bash = run(&w, &query(&["8080"]), &kinds(None, &["BASH"]), 5);
    assert_eq!((count(&bash[0], Kind::ToolInput), count(&bash[0], Kind::ToolOutput)), (Some(1), Some(1)));
    assert!(bash[0].snippets.iter().all(|s| s.tool.as_deref() == Some("Bash")));
    let mixed = run(&w, &query(&["run it|8080"]), &kinds(Some("prompt,tool-output"), &["Read"]), 5);
    assert_eq!((count(&mixed[0], Kind::Prompt), count(&mixed[0], Kind::ToolOutput)), (Some(1), Some(1)), "--tool narrows only the tool kinds");
}

#[test]
fn short_terms_match_like_long_ones_in_any_case() {
    let mut a = T::new(SA, "/Users/me/Code/edge");
    a.prompt("网络 断了").reply("Ab test, ÉCOLE");
    let mut b = T::new(SB, "/Users/me/Code/edge");
    b.prompt("没有关系").reply("école");
    let w = world(&[(&a, BASE_MS + DAY), (&b, BASE_MS + 2 * DAY)]);
    let k = kinds(None, &[]);
    assert_eq!(ids(&run(&w, &query(&["网络"]), &k, 3)), vec![SA], "two CJK characters");
    assert_eq!(ids(&run(&w, &query(&["aB"]), &k, 3)), vec![SA]);
    assert_eq!(ids(&run(&w, &query(&["éc"]), &k, 3)), vec![SB, SA], "non-ASCII case folds too");
    assert_eq!(ids(&run(&w, &query(&["网络", "ÉCOLE"]), &k, 3)), vec![SA]);
}

#[test]
fn quotes_backslashes_and_line_breaks_are_found_through_json_escapes() {
    let mut a = T::new(SA, "/Users/me/Code/edge");
    a.prompt("say \"hello\" to C:\\Users\\me")
        .reply("line one\nline two")
        .tool("toolu_1", "Bash", json!({"command": "printf 'tab\\there'"}), "col1\tcol2 \u{1b}[31mred\u{1b}[0m done");
    let w = world(&[(&a, BASE_MS + DAY)]);
    let all = kinds(Some("all"), &[]);
    assert_eq!(run(&w, &query(&["\"hello\""]), &all, 3).len(), 1);
    assert_eq!(run(&w, &query(&["C:\\Users"]), &all, 3).len(), 1);
    assert_eq!(run(&w, &query(&["one\nline"]), &all, 3).len(), 1);
    assert_eq!(run(&w, &query(&["col1\tcol2"]), &all, 3).len(), 1);
    assert_eq!(run(&w, &query(&["red\u{1b}[0m"]), &all, 3).len(), 1);
    assert_eq!(run(&w, &Query::parse(&terms(&["line one\\nline"]), true, false).unwrap(), &all, 3).len(), 1);
    assert_eq!(run(&w, &Query::parse(&terms(&["\"hel+o\""]), true, false).unwrap(), &all, 3).len(), 1);
    assert!(run(&w, &query(&["\\\"hello"]), &all, 3).is_empty(), "the escaped form is not the text");
}

#[test]
fn regexes_find_what_their_literals_point_to() {
    let mut a = T::new(SA, "/Users/me/Code/edge");
    a.prompt("connection reset by peer").reply("got 42 errors and Fatal: boom");
    let mut b = T::new(SB, "/Users/me/Code/edge");
    b.prompt("unrelated").reply("nothing here");
    let w = world(&[(&a, BASE_MS + DAY), (&b, BASE_MS + 2 * DAY)]);
    let k = kinds(None, &[]);
    for (re, expect) in [
        ("connection (refused|reset)", vec![SA]),
        ("\\d+ errors", vec![SA]),
        ("(?-i)Fatal", vec![SA]),
        ("(?-i)fatal", vec![]),
        ("[a-z]+ by peer", vec![SA]),
        ("\\w+ing", vec![SB]),
        ("über|unrelated", vec![SB]),
    ] {
        let q = Query::parse(&terms(&[re]), true, false).unwrap();
        assert_eq!(ids(&run(&w, &q, &k, 3)), expect, "{re}");
    }
}

#[test]
fn files_without_every_term_are_skipped_whole() {
    let mut a = T::new(SA, "/Users/me/Code/edge");
    a.prompt("alpha only");
    let mut b = T::new(SB, "/Users/me/Code/edge");
    b.prompt("alpha and beta");
    let w = world(&[(&a, BASE_MS + DAY), (&b, BASE_MS + 2 * DAY)]);
    let out = search(&w.db, &all_sessions(&w), &query(&["alpha", "beta"]), &kinds(Some("all"), &[]), &opts(None, 3)).unwrap();
    assert_eq!(out.engine, Engine::Scan);
    let stats = out.scan.unwrap();
    assert_eq!((stats.files, stats.skipped_files), (2, 1));
    assert!(stats.skipped_bytes > 0 && stats.skipped_bytes < stats.bytes);
    assert_eq!(ids(&out.sessions), vec![SB]);
}

// ---------------------------------------------------------------------------------------------
// Turn numbers

/// Turn numbers in snippets come from the index (prompt rows up to the item) or, when scanning,
/// from the shared record filter; both agree with each other on every tricky record.
#[test]
fn both_ways_number_turns_alike_on_the_turn_rule_fixtures() {
    let fixtures = common::fixture_root().parent().unwrap().to_path_buf();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("claude");
    let dir = root.join("projects").join("-tmp-turns");
    fs::create_dir_all(&dir).unwrap();
    for (i, name) in ["turn_rules.jsonl", "reader.jsonl", "conversation.jsonl"].iter().enumerate() {
        fs::copy(fixtures.join(name), dir.join(format!("0000000{i}-0000-4000-8000-000000000000.jsonl"))).unwrap();
    }
    let mut db = Db::open(&tmp.path().join("index.sqlite")).unwrap();
    Indexer::new(ClaudeDir::new(&root), None).scan(&mut db, &mut |_| {}).unwrap();
    let w = World { tmp, db };
    let sessions = all_sessions(&w);
    assert_eq!(sessions.len(), 3);
    let k = kinds(Some("prompt,reply,thinking,other"), &[]);
    let mut seen = 0;
    for term in ["的", "e", "问", "o", "a", "i", "t"] {
        let hits = run_on(&w, &sessions, &query(&[term]), &k, 1000);
        seen += hits.iter().map(|h| h.snippets.len()).sum::<usize>();
    }
    assert!(seen > 40, "covered {seen} items");
    let rules = sessions.iter().find(|s| s.id.starts_with("00000000")).unwrap();
    let hits = run_on(&w, std::slice::from_ref(rules), &query(&["提问"]), &k, 100);
    let turns: Vec<(Kind, Option<u32>, String)> = hits[0].snippets.iter().map(|s| (s.kind, s.turn, s.text.text.clone())).collect();
    // a record whose uuid a sidechain record, an earlier copy or an isMeta image took is other text
    assert_eq!(turns, vec![
        (Kind::Prompt, Some(2), "工具结果之后的同 uuid 提问".into()),
        (Kind::Prompt, Some(3), "重复写入的提问".into()),
        (Kind::Other, Some(2), "旁支之后的同 uuid 提问".into()),
        (Kind::Other, Some(3), "重复写入的提问".into()),
        (Kind::Other, Some(3), "图片之后的同 uuid 提问".into()),
    ]);
    assert_eq!(turn_count(&w.db, &rules.id).unwrap(), 8, "a local command and its output are two turns");
    assert_eq!(hits[0].turns, 8, "both ways count the session's turns too");
}

/// When the transcript has grown since it was indexed, the scan numbers turns by the file it read
/// and counts the session's turns there too, so no snippet's turn is above the count.
#[test]
fn the_scan_counts_turns_in_the_transcript_it_read() {
    let mut a = T::new(SA, "/Users/me/Code/edge");
    a.prompt("first wombat").reply("ok");
    let w = world(&[(&a, BASE_MS + DAY)]);
    a.prompt("second").prompt("third wombat");
    let path = w.claude_root().join("projects").join(key_of(&a.cwd)).join(format!("{SA}.jsonl"));
    fs::write(&path, a.text()).unwrap();
    let q = query(&["wombat"]);
    let k = kinds(None, &[]);
    let scan = search(&w.db, &all_sessions(&w), &q, &k, &opts(Some(Engine::Scan), 5)).unwrap().sessions;
    assert_eq!(scan[0].turns, 3);
    assert_eq!(scan[0].snippets.iter().map(|h| h.turn).collect::<Vec<_>>(), vec![Some(1), Some(3)]);
    let index = search(&w.db, &all_sessions(&w), &q, &k, &opts(Some(Engine::Index), 5)).unwrap().sessions;
    assert_eq!((index[0].turns, index[0].snippets.len()), (1, 1), "the index has not seen the new turns yet");
}

#[test]
fn text_before_the_first_prompt_has_no_turn() {
    let mut a = T::new(SA, "/Users/me/Code/edge");
    a.notification("early quokka").prompt("now the quokka question").reply("quokka answer");
    let w = world(&[(&a, BASE_MS + DAY)]);
    let hits = run(&w, &query(&["quokka"]), &kinds(Some("all"), &[]), 5);
    let turns: Vec<(Kind, Option<u32>)> = hits[0].snippets.iter().map(|s| (s.kind, s.turn)).collect();
    assert_eq!(turns, vec![(Kind::Prompt, Some(1)), (Kind::Reply, Some(1)), (Kind::Other, None)]);
    assert_eq!(turn_count(&w.db, SA).unwrap(), 1);
}

// ---------------------------------------------------------------------------------------------
// Which sessions are searched

fn filter_world() -> (World, PathBuf) {
    let mut a = T::new(SA, "/Users/me/Code/alpha").starting(BASE_MS);
    a.prompt("needle in alpha").reply("ok");
    let mut b = T::new(SB, "/Users/me/Code/alpha/sub").starting(BASE_MS + 10 * DAY);
    b.prompt("needle in sub").reply("ok");
    let mut c = T::new(SC, "/Users/me/Code/beta").starting(BASE_MS + 20 * DAY);
    c.prompt("needle in beta").reply("ok");
    let w = world(&[(&a, BASE_MS + 5 * DAY), (&b, BASE_MS + 11 * DAY), (&c, BASE_MS + 21 * DAY)]);
    let home = w.tmp.path().join("home");
    (w, home)
}

fn selected(w: &World, f: &SessionFilter) -> Vec<String> {
    select_sessions(&w.db, f).unwrap().into_iter().map(|s| s.id).collect()
}

#[test]
fn time_range_keeps_sessions_whose_activity_overlaps_it() {
    let (w, _) = filter_world();
    let range = |since: Option<i64>, until: Option<i64>| selected(&w, &SessionFilter { since_ms: since, until_ms: until, ..Default::default() });
    assert_eq!(range(None, None), vec![SC, SB, SA], "most recently active first");
    assert_eq!(range(Some(BASE_MS + 3 * DAY), Some(BASE_MS + 4 * DAY)), vec![SA], "SA was active from day 0 to day 5");
    assert_eq!(range(Some(BASE_MS + 6 * DAY), Some(BASE_MS + 10 * DAY)), Vec::<String>::new(), "SB's first prompt is at the end: excluded");
    assert_eq!(range(Some(BASE_MS + 11 * DAY), None), vec![SC, SB], "a last update right at the start counts");
    assert_eq!(range(None, Some(BASE_MS + DAY)), vec![SA]);
}

#[test]
fn a_session_without_a_first_prompt_spans_its_last_update() {
    let mut a = T::new(SA, "/Users/me/Code/alpha");
    a.reply("no prompt here");
    let w = world(&[(&a, BASE_MS + 5 * DAY)]);
    assert_eq!(all_sessions(&w)[0].started_at, None);
    let f = |since, until| selected(&w, &SessionFilter { since_ms: Some(since), until_ms: Some(until), ..Default::default() });
    assert_eq!(f(BASE_MS, BASE_MS + 5 * DAY), Vec::<String>::new());
    assert_eq!(f(BASE_MS, BASE_MS + 5 * DAY + 1), vec![SA]);
}

#[test]
fn projects_are_text_or_directories_with_the_projects_inside() {
    let (w, home) = filter_world();
    let cwd = Path::new("/Users/me/Code/alpha");
    let p = |args: &[&str]| {
        let projects = args.iter().map(|a| ProjectMatcher::parse(a, cwd, &home)).collect();
        selected(&w, &SessionFilter { projects, ..Default::default() })
    };
    assert_eq!(p(&["ALPHA"]), vec![SB, SA], "text: case-insensitive, in the path");
    assert_eq!(p(&["-code-beta"]), vec![SC], "text: in the encoded directory name");
    assert_eq!(p(&["/Users/me/Code/alpha"]), vec![SB, SA], "a directory takes the projects inside it");
    assert_eq!(p(&["/Users/me/Code/alpha/sub/"]), vec![SB], "a trailing slash does not matter");
    assert_eq!(p(&["/Users/me/Code/alp"]), Vec::<String>::new(), "directories compare whole components");
    assert_eq!(p(&["."]), vec![SB, SA], "relative to the current directory");
    assert_eq!(p(&["../beta"]), vec![SC]);
    assert_eq!(p(&["./sub/../sub"]), vec![SB]);
    assert_eq!(p(&["sub/"]), vec![SB], "a slash makes it a path");
    assert_eq!(p(&["beta", "/Users/me/Code/alpha/sub"]), vec![SC, SB], "several: any");
    assert_eq!(p(&["nothing-like-it"]), Vec::<String>::new());
    assert_eq!(p(&["/Users/me/Code/beta"]), vec![SC]);
    assert!(matches!(ProjectMatcher::parse("~/x", cwd, &home), ProjectMatcher::Dir { .. }));
    assert!(matches!(ProjectMatcher::parse("..", cwd, &home), ProjectMatcher::Dir { .. }));
    assert!(matches!(ProjectMatcher::parse("edge", cwd, &home), ProjectMatcher::Text(_)));
}

#[test]
fn project_directories_match_through_symlinks_on_either_side() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().canonicalize().unwrap().join("real");
    let link = tmp.path().canonicalize().unwrap().join("link");
    fs::create_dir_all(real.join("app")).unwrap();
    fs::create_dir_all(real.join("other")).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let mut a = T::new(SA, link.join("app").to_str().unwrap());
    a.prompt("hello");
    let mut b = T::new(SB, real.join("other").to_str().unwrap());
    b.prompt("hello");
    // directories deleted since (never created here): the part that still exists is resolved
    let mut c = T::new(SC, real.join("gone/deep").to_str().unwrap());
    c.prompt("hello");
    const SD: &str = "dddddddd-0000-4000-8000-00000000000d";
    let mut d = T::new(SD, link.join("gone2").to_str().unwrap());
    d.prompt("hello");
    let w = world(&[(&a, BASE_MS + DAY), (&b, BASE_MS + 2 * DAY), (&c, BASE_MS + 3 * DAY), (&d, BASE_MS + 4 * DAY)]);
    let home = tmp.path().join("home");
    let p = |arg: &Path| {
        let projects = vec![ProjectMatcher::parse(arg.to_str().unwrap(), Path::new("/"), &home)];
        selected(&w, &SessionFilter { projects, ..Default::default() })
    };
    assert_eq!(p(&real.join("app")), vec![SA], "written through the link, asked by the real path");
    assert_eq!(p(&link.join("other")), vec![SB], "written by the real path, asked through the link");
    assert_eq!(p(&link.join("gone")), vec![SC], "a deleted directory, asked through the link");
    assert_eq!(p(&real.join("gone2")), vec![SD], "a deleted directory written through the link");
    assert_eq!(p(&link), vec![SD, SC, SB, SA]);
}

#[test]
fn home_and_current_session_and_favorites_and_model_filters() {
    let (w, home) = filter_world();
    let tilde = ProjectMatcher::parse("~/Code", Path::new("/"), Path::new("/Users/me"));
    assert_eq!(selected(&w, &SessionFilter { projects: vec![tilde], ..Default::default() }), vec![SC, SB, SA]);
    let _ = home;
    assert_eq!(selected(&w, &SessionFilter { exclude: Some(SB.into()), ..Default::default() }), vec![SC, SA]);
    w.db.conn().execute("UPDATE sessions SET favorite = 1 WHERE id = ?1", [SA]).unwrap();
    assert_eq!(selected(&w, &SessionFilter { favorites: true, ..Default::default() }), vec![SA]);
    assert_eq!(selected(&w, &SessionFilter { model: Some("opus-5-5".into()), ..Default::default() }).len(), 3);
    assert_eq!(selected(&w, &SessionFilter { model: Some("claude-opus-5-5".into()), ..Default::default() }).len(), 3);
    assert!(selected(&w, &SessionFilter { model: Some("fable-5-2".into()), ..Default::default() }).is_empty());
}

#[test]
fn only_the_selected_sessions_are_searched_by_either_way() {
    let w = concepts_world();
    let sessions: Vec<SessionSummary> = all_sessions(&w).into_iter().filter(|s| s.id != SB).collect();
    let hits = run_on(&w, &sessions, &query(&["healthz"]), &kinds(None, &[]), 3);
    assert_eq!(ids(&hits), vec![SA]);
}

#[test]
fn results_sort_by_recent_activity_or_by_hits() {
    let mut a = T::new(SA, "/Users/me/Code/edge");
    a.prompt("token token").reply("token").reply("token");
    let mut b = T::new(SB, "/Users/me/Code/edge");
    b.prompt("token");
    let mut c = T::new(SC, "/Users/me/Code/edge");
    c.prompt("token").reply("token").reply("token");
    let w = world(&[(&a, BASE_MS + DAY), (&b, BASE_MS + 3 * DAY), (&c, BASE_MS + 2 * DAY)]);
    let (q, k) = (query(&["token"]), kinds(None, &[]));
    assert_eq!(ids(&run(&w, &q, &k, 3)), vec![SB, SC, SA], "recent activity by default");
    let sessions = all_sessions(&w);
    for engine in [Engine::Index, Engine::Scan] {
        let ask = |sort, limit| search(&w.db, &sessions, &q, &k, &SearchOptions { sort, limit, ..opts(Some(engine), 3) }).unwrap();
        let by_hits = ask(SortBy::Hits, 0);
        assert_eq!(ids(&by_hits.sessions), vec![SC, SA, SB], "most hits first, ties by recent activity");
        let top = ask(SortBy::Hits, 2);
        assert_eq!((top.total, ids(&top.sessions)), (3, vec![SC, SA]), "the limit cuts the sorted list; the total counts all");
        assert_eq!(top.sessions, by_hits.sessions[..2], "{engine:?}: the sessions shown are complete");
        let recent = ask(SortBy::Recent, 1);
        assert_eq!((recent.total, ids(&recent.sessions)), (3, vec![SB]));
    }
}

#[test]
fn a_regex_or_a_tool_kind_needs_the_scan_and_the_index_answers_the_rest() {
    let w = concepts_world();
    let sessions = all_sessions(&w);
    let engine = |q: &Query, k: &KindSet| search(&w.db, &sessions, q, k, &opts(None, 3)).unwrap().engine;
    assert_eq!(engine(&query(&["healthz"]), &kinds(Some("prompt,reply,thinking,other"), &[])), Engine::Index);
    assert_eq!(engine(&Query::parse(&terms(&["healthz"]), true, false).unwrap(), &kinds(None, &[])), Engine::Scan);
    assert_eq!(engine(&query(&["healthz"]), &kinds(Some("prompt,tool-output"), &[])), Engine::Scan);
    assert!(search(&w.db, &sessions, &Query::parse(&terms(&["x"]), true, false).unwrap(), &kinds(None, &[]), &opts(Some(Engine::Index), 3)).is_err());
}

// ---------------------------------------------------------------------------------------------
// Real data (`ASM_BENCH_DB=<a copy of a real index> cargo test --release -p agent-session-core
// --test search_sessions real_ -- --ignored --nocapture`)

fn bench_db() -> Db {
    let path = std::env::var("ASM_BENCH_DB").expect("set ASM_BENCH_DB to a copy of a real index");
    Db::open_read_only(Path::new(&path)).unwrap()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// `;` separates queries, `,` the TERMs of one.
fn bench_queries(var: &str, default: &str) -> Vec<Vec<String>> {
    let queries = std::env::var(var).unwrap_or_else(|_| default.to_string());
    queries.split(';').filter(|q| !q.is_empty()).map(|q| q.split(',').map(String::from).collect()).collect()
}

/// Index: common and ordinary words over all time (target 100 ms), each run on a new connection
/// as a one-off `asm search` would, the first run apart. Scan: the last 30 days with tool kinds,
/// literal (target 2 s), and how much of it the prefilter skipped whole.
#[test]
#[ignore]
fn real_search_timings() {
    let db = bench_db();
    let runs: usize = std::env::var("ASM_BENCH_RUNS").map_or(3, |r| r.parse().unwrap());
    // sessions shown, as `asm search -n` (default 20, 0 for all): only they get snippets
    let limit: usize = std::env::var("ASM_BENCH_LIMIT").map_or(20, |r| r.parse().unwrap());
    let all = select_sessions(&db, &SessionFilter::default()).unwrap();
    println!("sessions: {}", all.len());
    drop(db);
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
    for t in bench_queries("ASM_BENCH_INDEX", "error;healthz;连不上|无法连接|offline,healthz|健康检查;网络;session,cache;的;Ωx") {
        let q = Query::parse(&t, false, false).unwrap();
        let (mut select, mut total) = (Vec::new(), Vec::new());
        let mut out = None;
        for _ in 0..runs {
            let start = std::time::Instant::now();
            let db = bench_db();
            let sessions = select_sessions(&db, &SessionFilter::default()).unwrap();
            select.push(ms(start));
            let r = search(&db, &sessions, &q, &KindSet::default(), &SearchOptions { limit, ..SearchOptions::default() }).unwrap();
            total.push(ms(start));
            out = Some(r);
        }
        let out = out.unwrap();
        let items: u32 = out.sessions.iter().map(SessionHits::total).sum();
        let f = |v: &[f64]| v.iter().map(|t| format!("{t:.1}")).collect::<Vec<_>>().join("/");
        println!(
            "index {t:?} -n {limit}: first {:.1} ms, median {:.1} ms (runs {}; open+select_sessions {}), {} sessions found, {items} items in those shown",
            total[0],
            median(total.clone()),
            f(&total),
            f(&select),
            out.total
        );
    }
    let db = bench_db();
    let now = chrono::Utc::now().timestamp_millis();
    let filter = SessionFilter { since_ms: Some(now - 30 * DAY), ..Default::default() };
    for t in bench_queries("ASM_BENCH_SCAN", "database is locked;connection refused;error;healthz,连不上|无法连接|offline") {
        let q = Query::parse(&t, false, false).unwrap();
        let mut times = Vec::new();
        let mut out = None;
        for _ in 0..runs {
            let start = std::time::Instant::now();
            let sessions = select_sessions(&db, &filter).unwrap();
            let r = search(&db, &sessions, &q, &KindSet::parse(Some("all"), &[]).unwrap(), &SearchOptions::default()).unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            out = Some(r);
        }
        let out = out.unwrap();
        let st = out.scan.unwrap();
        println!(
            "scan 30d all kinds {t:?}: median {:.0} ms ({:?}), {} sessions; files {} skipped {} ({:.0}%), bytes {:.2} GB skipped {:.2} GB ({:.0}%)",
            median(times.clone()),
            times.iter().map(|t| format!("{t:.0}")).collect::<Vec<_>>(),
            out.sessions.len(),
            st.files,
            st.skipped_files,
            100.0 * st.skipped_files as f64 / st.files.max(1) as f64,
            st.bytes as f64 / 1e9,
            st.skipped_bytes as f64 / 1e9,
            100.0 * st.skipped_bytes as f64 / st.bytes.max(1) as f64,
        );
    }
}

/// On every real session not written since the index was built, the index and the scan count
/// the same items and show the same snippets with the same turn numbers.
#[test]
#[ignore]
fn real_index_and_scan_agree() {
    let db = bench_db();
    let unchanged: Vec<SessionSummary> = select_sessions(&db, &SessionFilter::default())
        .unwrap()
        .into_iter()
        .filter(|s| {
            let Some(path) = s.file_path.as_deref() else { return false };
            fs::metadata(path).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .is_some_and(|t| t.as_millis() as i64 == s.updated_at)
        })
        .collect();
    println!("{} unchanged sessions", unchanged.len());
    let k = KindSet::parse(Some("prompt,reply,thinking,other"), &[]).unwrap();
    let mut differ = 0;
    for t in bench_queries("ASM_BENCH_AGREE", "error;的;cargo test;连不上|无法连接|offline,healthz|健康检查;Request interrupted;local-command-stdout;task-notification;thinking,cache") {
        for same in [false, true] {
            let q = Query::parse(&t, false, same).unwrap();
            let index = search(&db, &unchanged, &q, &k, &opts(Some(Engine::Index), 5)).unwrap().sessions;
            let scan = search(&db, &unchanged, &q, &k, &opts(Some(Engine::Scan), 5)).unwrap().sessions;
            let by_id: HashMap<&str, &SessionHits> = scan.iter().map(|h| (h.session_id.as_str(), h)).collect();
            let mut bad = 0;
            for h in &index {
                if by_id.get(h.session_id.as_str()) != Some(&h) {
                    bad += 1;
                    if bad <= 3 {
                        println!("  differ {}:\n    index {h:?}\n    scan  {:?}", h.session_id, by_id.get(h.session_id.as_str()));
                    }
                }
            }
            bad += scan.len().saturating_sub(index.len());
            println!("{t:?} same_message={same}: index {} sessions, scan {} sessions, {bad} differ", index.len(), scan.len());
            differ += bad;
        }
    }
    assert_eq!(differ, 0);
}


/// On the last 30 days of real transcripts (those not written in the last minute, so that both
/// runs read the same bytes), the byte prefilter does not change any result of a tool search.
/// `ASM_BENCH_TOOLS` overrides the queries: `;` between them, `,` between TERMs, a leading `re:`
/// for a regex query, `@Name ` before the TERMs for `--tool Name`.
#[test]
#[ignore]
fn real_prefilter_keeps_tool_results() {
    let db = bench_db();
    let now = chrono::Utc::now().timestamp_millis();
    let sessions: Vec<SessionSummary> = select_sessions(&db, &SessionFilter { since_ms: Some(now - 30 * DAY), ..Default::default() })
        .unwrap()
        .into_iter()
        .filter(|s| {
            let Some(path) = s.file_path.as_deref() else { return false };
            fs::metadata(path).ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 60)
        })
        .collect();
    println!("{} sessions of the last 30 days, not written in the last minute", sessions.len());
    let default = "file_path:;limit: 2000;$ git;#;error;connection refused;连不上|无法连接;re:file_path:.*config;re:\\$ cargo (test|build);@Bash git push;@Read file_path:;old_string,new_string";
    let mut differ = 0;
    for spec in std::env::var("ASM_BENCH_TOOLS").unwrap_or_else(|_| default.to_string()).split(';') {
        let (regex, spec) = match spec.strip_prefix("re:") {
            Some(rest) => (true, rest),
            None => (false, spec),
        };
        let (tools, spec) = match spec.strip_prefix('@') {
            Some(rest) => {
                let (tool, rest) = rest.split_once(' ').unwrap();
                (vec![tool.to_string()], rest)
            }
            None => (Vec::new(), spec),
        };
        let q = Query::parse(&spec.split(',').map(String::from).collect::<Vec<_>>(), regex, false).unwrap();
        let k = if tools.is_empty() { KindSet::parse(Some("tool"), &[]).unwrap() } else { KindSet::parse(None, &tools).unwrap() };
        let run = |prefilter| {
            let start = std::time::Instant::now();
            let o = search(&db, &sessions, &q, &k, &SearchOptions { prefilter, ..opts(Some(Engine::Scan), 5) }).unwrap();
            (o, start.elapsed().as_millis())
        };
        let ((on, on_ms), (off, off_ms)) = (run(true), run(false));
        let st = on.scan.as_ref().unwrap();
        let by_id: HashMap<&str, &SessionHits> = off.sessions.iter().map(|h| (h.session_id.as_str(), h)).collect();
        let mut bad = on.sessions.iter().filter(|h| by_id.get(h.session_id.as_str()) != Some(h)).count();
        bad += off.sessions.len().saturating_sub(on.sessions.len());
        let items: u32 = off.sessions.iter().map(SessionHits::total).sum();
        println!(
            "{}{:?}{}: prefilter on {} sessions ({on_ms} ms, {} of {} files skipped whole), off {} sessions / {items} items ({off_ms} ms), {bad} differ",
            if regex { "regex " } else { "" },
            q.terms().iter().map(|t| t.alternatives.join("|")).collect::<Vec<_>>(),
            if tools.is_empty() { String::new() } else { format!(" --tool {}", tools.join(",")) },
            on.sessions.len(),
            st.skipped_files,
            st.files,
            off.sessions.len(),
        );
        differ += bad;
    }
    assert_eq!(differ, 0);
}
