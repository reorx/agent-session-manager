//! Optional real-data audit. Expected verdicts are never read here: the reference TSV is
//! compared externally. Frozen call facts independently exercise Reader's input extraction.
use std::{collections::HashMap, fs::File, hash::{DefaultHasher, Hash, Hasher}, io::{BufRead, BufReader, Read, Write}, path::PathBuf};
use agent_session_core::{claude::{cache::{CacheTracker, CallInput, RequestFacts}, conversation::read_conversation}, model::{CacheBreak, CacheBreakCause, ConvItem, Tokens}};
use serde::Deserialize;

#[derive(Deserialize)]
struct Manifest { path: PathBuf, size: u64, sha256: String }
#[derive(Deserialize)]
struct Usage { input: u64, read: u64, write5: u64, write1: u64, output: u64 }
#[derive(Deserialize)]
struct Request { start: Option<i64>, shape: Option<String>, betas: Option<Vec<String>>, keep: Option<u32>, prev_len: Option<u32> }
#[derive(Deserialize)]
struct Call {
    session: String, id: String, ts: Option<i64>, model: String, effort: Option<String>,
    speed: Option<String>, parent: Option<String>, request: Option<Request>, tokens: Usage,
    prompt: u64, read: u64, events: Vec<(usize, String)>,
}
fn hash(value: &impl Hash) -> u64 {
    let mut h = DefaultHasher::new(); value.hash(&mut h); h.finish()
}
impl Call {
    fn input(&self) -> CallInput {
        let request = self.request.as_ref().map(|r| {
            let betas = r.betas.as_ref().map(|b| {
                let mut b: Vec<_> = b.iter().collect(); b.sort_unstable(); b.dedup(); b
            });
            RequestFacts { shape: r.shape.as_ref().map(hash), betas: betas.as_ref().map(hash),
                sign_in: betas.as_ref().map(|b| b.iter().any(|s| s.starts_with("oauth-"))), keep: r.keep, prev_len: r.prev_len }
        });
        CallInput { msg_id: self.id.clone(), parent: self.parent.clone(), ts: self.ts,
            start_ts: self.request.as_ref().and_then(|r| r.start), model: self.model.clone(),
            effort: self.effort.clone(), speed: self.speed.clone(), request,
            tokens: Tokens { input: self.tokens.input, output: self.tokens.output, cache_read: self.tokens.read,
                cache_write_5m: self.tokens.write5, cache_write_1h: self.tokens.write1, ..Tokens::default() },
            prompt: self.prompt, read: self.read }
    }
}

fn cause(c: &CacheBreakCause) -> &'static str {
    match c {
        CacheBreakCause::Compaction => "compaction", CacheBreakCause::ModelSwitch { .. } => "model",
        CacheBreakCause::EffortChange { .. } => "effort", CacheBreakCause::FastMode { .. } => "fast_mode",
        CacheBreakCause::RequestSettings { sign_in: true } => "sign_in",
        CacheBreakCause::RequestSettings { sign_in: false } => "request_settings",
        CacheBreakCause::Idle { .. } => "idle", CacheBreakCause::SystemOrTools => "system_or_tools",
        CacheBreakCause::EarlierMessages => "earlier_messages", CacheBreakCause::RequestUnchanged => "request_unchanged",
        CacheBreakCause::Unknown => "unknown",
    }
}

// Non-idle gaps/TTLs are not part of CacheBreak's public output. Derive these diagnostic-only
// columns from input facts; for every Idle verdict require equality with the production values.
fn ttl(segments: &[(u64, i64)], read: u64) -> i64 {
    segments.iter().find(|s| s.0 > read).or_else(|| segments.last()).map_or(300_000, |s| s.1)
}
fn prefix(previous: &[(u64, i64)], c: &Call) -> Vec<(u64, i64)> {
    let mut out: Vec<_> = previous.iter().copied().filter(|s| s.0 <= c.read).collect();
    if c.read > out.last().map_or(0, |s| s.0) { out.push((c.read, ttl(previous, c.read - 1))); }
    let mut end = c.read;
    for (n, t) in [(c.tokens.write1, 3_600_000), (c.tokens.write5, 300_000)] {
        if n > 0 { end += n; out.push((end, t)); }
    }
    out
}
fn optional(n: Option<i64>) -> String { n.map_or_else(String::new, |n| n.to_string()) }

fn audit_calls(calls: &[Call], actual: &[CacheBreak], out: &mut impl Write) {
    let mut tracker = CacheTracker::default();
    let mut seen: HashMap<&str, (usize, Vec<(u64, i64)>)> = HashMap::new();
    let mut last = None;
    let mut replay = Vec::new();
    let mut rows = Vec::new();
    for (i, c) in calls.iter().enumerate() {
        assert!(!seen.contains_key(c.id.as_str()), "duplicate frozen call {}", c.id);
        for (_, event) in &c.events {
            match event.as_str() { "compact" => tracker.compacted(), "turn" => tracker.new_turn(), _ => panic!("unknown event {event}") }
        }
        let p = c.parent.as_deref().and_then(|id| seen.get(id)).or_else(|| last.and_then(|id| seen.get(id)));
        let segments = p.map_or(&[][..], |p| p.1.as_slice());
        let gap = p.and_then(|(idx, _)| {
            let p = &calls[*idx];
            c.request.as_ref().and_then(|r| r.start).zip(p.request.as_ref().and_then(|r| r.start))
                .or_else(|| c.ts.zip(p.ts)).map(|(a, b)| a - b)
        });
        let lifetime = ttl(segments, c.read);
        if let Some(b) = tracker.call(&c.input()) {
            if let CacheBreakCause::Idle { gap_ms, ttl_ms } = &b.cause {
                assert_eq!((Some(*gap_ms), *ttl_ms), (gap, lifetime), "{} {}", c.session, c.id);
            }
            rows.push(format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n", c.session, optional(b.ts), cause(&b.cause), b.dropped, b.written, optional(gap), lifetime, c.id));
            replay.push(b);
        }
        let next = prefix(segments, c);
        seen.insert(&c.id, (i, next)); last = Some(c.id.as_str());
    }
    assert_eq!(actual, replay, "Reader vs frozen-fact replay: {}", calls.first().map_or("empty", |c| c.session.as_str()));
    for row in rows { out.write_all(row.as_bytes()).unwrap(); }
}

pub fn run() {
    let env = |key| PathBuf::from(std::env::var(key).unwrap_or_else(|_| panic!("set {key}")));
    let manifest: Vec<Manifest> = serde_json::from_reader(File::open(env("ASM_CACHE_AUDIT_MANIFEST")).unwrap()).unwrap();
    let mut facts = HashMap::new();
    for line in BufReader::new(File::open(env("ASM_CACHE_AUDIT_FACTS")).unwrap()).lines() {
        let calls: Vec<Call> = serde_json::from_str(&line.unwrap()).unwrap();
        if let Some(c) = calls.first() { assert!(facts.insert(c.session.clone(), calls).is_none()); }
    }
    let scratch = env("ASM_CACHE_AUDIT_SCRATCH"); std::fs::create_dir_all(&scratch).unwrap();
    let prefix_path = scratch.join("frozen-prefix.jsonl");
    let mut out = File::create(env("ASM_CACHE_AUDIT_OUT")).unwrap();
    writeln!(out, "session\treply_ms\tcause\tdropped\twritten\tgap_ms\tttl_ms\tmessage_id").unwrap();
    let mut total = 0;
    for (i, entry) in manifest.iter().enumerate() {
        let mut bytes = Vec::new();
        File::open(&entry.path).unwrap().take(entry.size).read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes.len() as u64, entry.size, "short file {}", entry.path.display());
        std::fs::write(&prefix_path, &bytes).unwrap();
        let digest = std::process::Command::new("shasum").args(["-a", "256"]).arg(&prefix_path).output().unwrap();
        assert!(digest.status.success());
        assert_eq!(String::from_utf8(digest.stdout).unwrap().split_whitespace().next().unwrap(), entry.sha256, "changed prefix {}", entry.path.display());
        let conv = read_conversation(&prefix_path, None).unwrap();
        assert_eq!(conv.skipped_lines, 0, "{}", entry.path.display());
        let actual: Vec<_> = conv.items.into_iter().flat_map(|item| match item { ConvItem::Turn(t) => t.cache_breaks, _ => vec![] }).collect();
        let session = entry.path.file_stem().unwrap().to_str().unwrap();
        let calls = facts.remove(session).unwrap_or_default();
        audit_calls(&calls, &actual, &mut out);
        total += actual.len();
        if i % 100 == 0 { eprintln!("audited {}/{} files, {total} rebuilds", i + 1, manifest.len()); }
    }
    assert!(facts.is_empty(), "facts absent from manifest");
    std::fs::remove_file(prefix_path).unwrap();
    eprintln!("audited {} SHA256-verified prefixes, {total} rebuilds", manifest.len());
}

/// Verify UI aggregation against real ledger backups, never the default data directory.
pub fn ledger_correction() {
    use agent_session_core::{ledger::Ledger, usage::{UsageData, usage_day, day_start_ms}};
    use chrono::FixedOffset;
    let before_path = PathBuf::from(std::env::var("ASM_CACHE_LEDGER_BEFORE").expect("set ASM_CACHE_LEDGER_BEFORE"));
    let after_path = PathBuf::from(std::env::var("ASM_CACHE_LEDGER_AFTER").expect("set ASM_CACHE_LEDGER_AFTER"));
    let out = PathBuf::from(std::env::var("ASM_CACHE_LEDGER_OUT").expect("set ASM_CACHE_LEDGER_OUT"));
    let before = Ledger::open(&before_path).unwrap();
    let after = Ledger::open(&after_path).unwrap();
    let cases = [
        ("01dacff6-a59f-4b3c-8e6c-a0d8f093fad7", 1_793_720_686_324, 0.0773524),
        ("196366dd-076b-4651-890b-767be310162a", 1_794_722_605_922, 0.0770456),
        ("603da7ec-61df-4901-a1e1-ec268c219348", 1_794_215_685_957, 0.0780306),
        ("d074913f-7c07-4462-b44f-d3b29c511d5e", 1_793_785_422_630, 0.1411698),
    ];
    let mut rows = Vec::new();
    let tz = FixedOffset::east_opt(8 * 3600).unwrap();
    for (id, ts, expected) in cases {
        let ids = [id.to_owned()];
        let mut a = UsageData::default(); a.replace_all(before.usage(Some(&ids)).unwrap());
        let mut b = UsageData::default(); b.replace_all(after.usage(Some(&ids)).unwrap());
        let day = usage_day(ts, &tz);
        let start = day_start_ms(day, &tz);
        let end = day_start_ms(day.succ_opt().unwrap(), &tz);
        let old = a.totals(start, end); let new = b.totals(start, end);
        let delta = new.cost.total() - old.cost.total();
        assert!((delta - expected).abs() < 1e-8, "{id}: {delta} != {expected}");
        assert_eq!(old.responses, new.responses, "no new responses in this session/day");
        rows.push(serde_json::json!({"session":id,"usage_day":day.to_string(),"start_ms":start,"end_ms":end,
            "before_usd":old.cost.total(),"after_usd":new.cost.total(),"delta_usd":delta,
            "delta_input":new.tokens.input-old.tokens.input,"delta_read":new.tokens.cache_read-old.tokens.cache_read,
            "delta_output":new.tokens.output-old.tokens.output,"responses":new.responses}));
    }
    std::fs::write(out, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
}
