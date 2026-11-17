//! Cache command contracts: independent billing/cache facts, lifetime detail and call ranges.
use assert_cmd::Command;
use serde_json::{Value, json};
use tempfile::TempDir;

struct Data { tmp: TempDir }
impl Data {
    fn new() -> Self {
        let d = Self { tmp: tempfile::tempdir().unwrap() };
        std::fs::create_dir_all(d.tmp.path().join("claude/projects/-Users-me-Code-homehub")).unwrap();
        let now = chrono::Utc::now();
        let old = now - chrono::Duration::days(90);
        for id in ["aaa-one", "aaa-two"] {
            let records = [
                json!({"type":"user","uuid":"u","message":{"role":"user","content":"hello"},"timestamp":old.to_rfc3339(),"cwd":"/Users/me/Code/homehub"}),
                call("a", "u", old, "claude-sonnet-4-5", 30000, 0),
                call("b", "a", now, "claude-sonnet-4-5", 30000, 0),
                call("c", "b", now, "mystery", 30000, 0),
            ];
            d.write(id, &records);
        }
        let sub = d.tmp.path().join("claude/projects/-Users-me-Code-homehub/aaa-one/subagents");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("agent-x.jsonl"), format!("{}\n", call("sub", "u", now, "claude-haiku-4-5", 0, 10000))).unwrap();
        d.write("zero", &[call("z", "missing", now, "mystery", 0, 0)]);
        d
    }
    fn write(&self, id: &str, records: &[Value]) {
        let text = records.iter().map(|r| format!("{r}\n")).collect::<String>();
        std::fs::write(self.tmp.path().join(format!("claude/projects/-Users-me-Code-homehub/{id}.jsonl")), text).unwrap();
    }
    fn cmd(&self) -> Command {
        let mut c = Command::cargo_bin("asm").unwrap();
        c.env("ASM_CLAUDE_DIR", self.tmp.path().join("claude"))
            .env("ASM_DB", self.tmp.path().join("index.sqlite"))
            .env("ASM_STATE", self.tmp.path().join("state.json"))
            .env("ASM_FAVORITES_DIR", self.tmp.path().join("favorites"))
            .env_remove("CLAUDE_CODE_SESSION_ID").env("TZ", "UTC");
        c
    }
    fn json(&self, args: &[&str]) -> Value {
        let o = self.cmd().args(args).arg("--json").output().unwrap();
        assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
        serde_json::from_slice(&o.stdout).unwrap()
    }
}
fn call(id: &str, parent: &str, time: chrono::DateTime<chrono::Utc>, model: &str, write: u64, read: u64) -> Value {
    json!({"type":"assistant","uuid":id,"parentUuid":parent,"timestamp":time.to_rfc3339(),
        "message":{"id":id,"role":"assistant","model":model,"content":[{"type":"text","text":"ok"}],
        "usage":{"input_tokens":0,"output_tokens":1,"cache_creation_input_tokens":write,"cache_read_input_tokens":read}}})
}

#[test]
fn overview_defaults_to_call_time_last_week_and_subagents_only_add_tokens() {
    let d = Data::new();
    let v = d.json(&["cache"]);
    assert_eq!(v["meta"]["range"]["label"], "last 7 days (default)");
    assert_eq!(v["totals"]["calls"], 6);
    assert_eq!(v["totals"]["main_thread_calls"], 5);
    assert_eq!(v["totals"]["input_tokens"], 130000);
    assert_eq!(v["totals"]["rebuilds"], 4);
    assert!(v["totals"]["cost_usd"].is_null());
    assert_eq!(v["totals"]["unpriced_rebuilds"], 2);
    assert_eq!(v["meta"]["total"], 3);
    let all = d.json(&["cache", "--days", "0", "--limit", "1", "--by", "model"]);
    assert_eq!(all["totals"]["calls"], 8);
    assert_eq!(all["totals"]["input_tokens"], 190000);
    assert_eq!(all["results"].as_array().unwrap().len(), 1);
    assert_eq!(all["meta"]["total"], 3);
    assert_eq!(all["meta"]["truncated"], true);
    assert_eq!(all["groups"].as_array().unwrap().len(), 3);
}

#[test]
fn models_filter_calls_not_primary_models_and_groups_do_not_multiply_facts() {
    let d = Data::new();
    let v = d.json(&["cache", "--model", "haiku-4-5", "--by", "model"]);
    assert_eq!(v["totals"]["input_tokens"], 10000);
    assert_eq!(v["totals"]["main_thread_calls"], 0);
    assert_eq!(v["totals"]["rebuilds"], 0);
    assert_eq!(v["totals"]["cost_usd"], 0.0);
    assert_eq!(v["groups"][0]["key"], "claude-haiku-4-5");
    assert_eq!(v["groups"][0]["stats"], v["totals"]);
    for by in ["day", "project"] {
        let v = d.json(&["cache", "--by", by, "--project", "homehub"]);
        assert_eq!(v["groups"].as_array().unwrap().len(), 1);
        assert_eq!(v["groups"][0]["stats"], v["totals"]);
    }
    let empty = d.json(&["cache", "--project", "no-such-project"]);
    assert_eq!(empty["meta"]["total"], 0);
    assert!(empty["totals"]["hit_rate"].is_null());
}

#[test]
fn lifetime_detail_matches_session_hit_and_keeps_nullable_turns_and_calls() {
    let d = Data::new();
    let v = d.json(&["cache", "aaa-one", "--calls"]);
    let s = d.json(&["session", "aaa-one"]);
    assert_eq!(v["totals"]["hit_rate"], s["session"]["tokens"]["hit_rate"]);
    assert_eq!(v["calls"].as_array().unwrap().len(), 3);
    assert_eq!(v["rebuilds"].as_array().unwrap().len(), 2);
    assert_eq!(v["calls"][1]["turn"], 1);
    assert_eq!(v["calls"][1]["rebuild"], true);
    assert_eq!(v["rebuilds"][1]["cause"]["kind"], "model_switch");
    let z = d.json(&["cache", "zero", "--calls"]);
    assert!(z["calls"][0]["turn"].is_null());
    assert!(z["totals"]["hit_rate"].is_null());
    assert!(d.json(&["cache", "aaa-one"])["calls"].as_array().unwrap().is_empty());
}

#[test]
fn lookup_errors_invalid_combinations_and_empty_ranges_have_exact_statuses() {
    let d = Data::new();
    for (id, code) in [("aaa", "ambiguous"), ("missing", "not_found")] {
        let o = d.cmd().args(["cache", id, "--json"]).output().unwrap();
        assert_eq!(o.status.code(), Some(1));
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(v["error"]["code"], code);
    }
    for flags in [vec!["--days", "0"], vec!["--since", "2026-01-01"], vec!["--until", "2026-01-01"],
        vec!["--by", "model"], vec!["--model", "mystery"], vec!["--project", "homehub"],
        vec!["--sort", "rebuild-cost"], vec!["--limit", "10"], vec!["--include-self"]] {
        let o = d.cmd().args(["cache", "aaa-one"]).args(flags).output().unwrap();
        assert_eq!(o.status.code(), Some(2));
    }
    assert_eq!(d.cmd().args(["cache", "--calls"]).output().unwrap().status.code(), Some(2));
    let v = d.json(&["cache", "--until", "2000-01-01"]);
    assert_eq!(v["totals"]["calls"], 0);
    let o = d.cmd().args(["cache", "--until", "2000-01-01"]).output().unwrap();
    assert!(String::from_utf8_lossy(&o.stdout).contains("0 calls in range"));
}

#[test]
fn ranking_limit_self_exclusion_and_refresh_fallback() {
    let d = Data::new();
    d.json(&["cache"]);
    let db = rusqlite::Connection::open(d.tmp.path().join("index.sqlite")).unwrap();
    db.execute("UPDATE sessions SET updated_at=1", []).unwrap();
    for sort in ["hit", "tokens"] {
        let v = d.json(&["cache", "--sort", sort, "--no-scan"]);
        let ids: Vec<_> = v["results"].as_array().unwrap().iter().map(|r| r["session_id"].as_str().unwrap()).collect();
        let expected = if sort == "hit" { vec!["aaa-two", "aaa-one", "zero"] } else { vec!["aaa-one", "aaa-two", "zero"] };
        assert_eq!(ids, expected);
        assert_eq!(v["meta"]["index_refresh"], "skipped");
    }
    let o = d.cmd().env("CLAUDE_CODE_SESSION_ID", "aaa-one").args(["cache", "--json", "--no-scan"]).output().unwrap();
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["meta"]["total"], 2);
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let v = d.json(&["cache"]);
    assert_eq!(v["meta"]["index_refresh"], "busy");
    db.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn local_day_boundaries_unknown_times_and_ties_are_stable() {
    let d = Data::new();
    d.json(&["cache"]);
    let db = rusqlite::Connection::open(d.tmp.path().join("index.sqlite")).unwrap();
    // Billing timestamps: unknown, exactly midnight, and exactly the next midnight.
    db.execute("UPDATE api_usage SET ts=0", []).unwrap();
    let midnight = 1788220800000_i64; // 2026-09-01 UTC
    db.execute("UPDATE api_usage SET ts=?1 WHERE msg_id='b'", [midnight]).unwrap();
    db.execute("UPDATE api_usage SET ts=?1 WHERE msg_id='c'", [midnight + 86400000]).unwrap();
    let v = d.json(&["cache", "--since", "2026-09-01", "--until", "2026-09-01", "--by", "day", "--no-scan"]);
    assert_eq!(v["totals"]["calls"], 2);
    assert_eq!(v["groups"][0]["key"], "2026-09-01");
    let all = d.json(&["cache", "--days", "0", "--by", "day", "--no-scan"]);
    assert_eq!(all["totals"]["calls"], 8);
    assert!(all["groups"].as_array().unwrap().last().unwrap()["key"].is_null());
    // Exact ranking ties, including metadata time, fall back to ID ascending.
    db.execute("DELETE FROM api_usage WHERE agent != ''", []).unwrap();
    db.execute("UPDATE sessions SET updated_at=1", []).unwrap();
    for sort in ["tokens", "hit", "rebuild-cost"] {
        let v = d.json(&["cache", "--days", "0", "--sort", sort, "--no-scan"]);
        let ids: Vec<_> = v["results"].as_array().unwrap().iter().map(|r| r["session_id"].as_str().unwrap()).collect();
        let expected = if sort == "rebuild-cost" { vec!["zero", "aaa-one", "aaa-two"] } else { vec!["aaa-one", "aaa-two", "zero"] };
        assert_eq!(ids, expected);
    }
}

#[test]
fn absent_transcripts_history_and_no_scan_cold_start_still_have_defined_results() {
    let d = Data::new();
    let initial = d.json(&["cache", "aaa-one", "--calls", "--no-scan"]);
    assert_eq!(initial["meta"]["index_refresh"], "done");
    std::fs::remove_file(d.tmp.path().join("claude/projects/-Users-me-Code-homehub/aaa-one.jsonl")).unwrap();
    let missing = d.json(&["cache", "aaa-one", "--calls"]);
    assert_eq!(missing["totals"], initial["totals"]);
    assert_eq!(missing["rebuilds"], initial["rebuilds"]);
    let db = rusqlite::Connection::open(d.tmp.path().join("index.sqlite")).unwrap();
    db.execute("INSERT INTO sessions(id, project_id, source) SELECT 'history-test', project_id, 'history' FROM sessions LIMIT 1", []).unwrap();
    let h = d.json(&["cache", "history-test", "--calls", "--no-scan"]);
    assert_eq!(h["totals"]["calls"], 0);
    assert!(h["calls"].as_array().unwrap().is_empty());
    assert!(h["rebuilds"].as_array().unwrap().is_empty());
    let o = d.cmd().env("ASM_DB", d.tmp.path()).args(["cache", "--json"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["error"]["code"], "failed");
}

#[test]
fn include_self_and_project_union_do_not_change_lifetime_or_duplicate_totals() {
    let d = Data::new();
    let expected = d.json(&["cache", "--project", "homehub"]);
    let union = d.json(&["cache", "--project", "homehub", "--project", "/Users/me/Code/homehub"]);
    assert_eq!(union["totals"], expected["totals"]);
    let o = d.cmd().env("CLAUDE_CODE_SESSION_ID", "aaa-one").args(["cache", "--include-self", "--json"]).output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["totals"], expected["totals"]);
    let o = d.cmd().env("CLAUDE_CODE_SESSION_ID", "aaa-one").args(["cache", "aaa-one", "--json"]).output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["totals"]["calls"], 4);
}

#[test]
fn auto_calls_never_inherit_the_previous_numbered_turn() {
    let d = Data::new();
    let now = chrono::Utc::now();
    d.write("auto", &[
        json!({"type":"user","uuid":"u","message":{"content":"prompt"}}),
        call("a", "u", now, "claude-sonnet-4-5", 30000, 0),
        json!({"type":"user","uuid":"auto","parentUuid":"a","message":{"content":"<local-command-stdout>done</local-command-stdout>"}}),
        call("b", "auto", now, "claude-sonnet-4-5", 30000, 0),
    ]);
    let v = d.json(&["cache", "auto", "--calls"]);
    assert_eq!(v["calls"][0]["turn"], 1);
    assert!(v["calls"][1]["turn"].is_null());
    assert!(v["rebuilds"][0]["turn"].is_null());
    let text = d.cmd().args(["cache", "auto", "--calls"]).output().unwrap();
    assert_eq!(text.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&text.stdout).contains("turn —"));
}
