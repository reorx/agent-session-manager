//! Cache rebuild detection independent of transcript records.
use std::collections::HashMap;
use crate::model::{CacheBreak, CacheBreakCause, Tokens};
use crate::pricing::{LONG_PROMPT, price, price_tokens};

/// Minimum missing prefix size; smaller drops are ordinary cache boundary differences.
pub const CACHE_BREAK_MIN: u64 = 10_000;
const FIVE_MINUTES: i64 = 300_000;
const HOUR: i64 = 3_600_000;

#[derive(Debug, Clone)]
pub struct CallInput {
    pub msg_id: String,
    /// Nearest ancestor response; absent or unknown falls back to file order.
    pub parent: Option<String>,
    /// Reply time used to place the note on the conversation.
    pub ts: Option<i64>,
    pub start_ts: Option<i64>,
    pub model: String,
    pub effort: Option<String>,
    pub speed: Option<String>,
    /// All billed iterations; prompt/read below describe only the final message.
    pub tokens: Tokens,
    pub prompt: u64,
    pub read: u64,
    pub request: Option<RequestFacts>,
}

/// Missing fields remain unknown; empty beta lists are distinct from absent lists.
#[derive(Debug, Clone, Default)]
pub struct RequestFacts {
    pub shape: Option<u64>,
    pub betas: Option<u64>,
    pub sign_in: Option<bool>,
    pub keep: Option<u32>,
    pub prev_len: Option<u32>,
}

struct CallState {
    prompt: u64,
    model: String,
    effort: Option<String>,
    speed: Option<String>,
    ts: Option<i64>,
    start_ts: Option<i64>,
    request: Option<RequestFacts>,
    segments: Vec<(u64, i64)>,
}

#[derive(Default)]
pub struct CacheTracker {
    calls: HashMap<String, CallState>,
    last: Option<String>,
    compacted: bool,
    called_since_compact: bool,
}

impl CacheTracker {
    pub(crate) fn contains(&self, id: &str) -> bool { self.calls.contains_key(id) }

    pub fn compacted(&mut self) {
        self.compacted = true;
        self.called_since_compact = false;
    }

    /// A manual compact followed by a user prompt must still mark its first rebuild.
    pub fn new_turn(&mut self) {
        if self.called_since_compact { self.compacted = false; }
    }

    pub fn call(&mut self, c: &CallInput) -> Option<CacheBreak> {
        if self.calls.contains_key(&c.msg_id) { return None; }
        let previous = c.parent.as_ref().and_then(|id| self.calls.get(id))
            .or_else(|| self.last.as_ref().and_then(|id| self.calls.get(id)));
        let segments = previous.map_or(&[][..], |p| p.segments.as_slice());
        let result = previous.and_then(|p| {
            let dropped = p.prompt.saturating_sub(c.read);
            if dropped < CACHE_BREAK_MIN { return None; }
            let cause = cause_for(c, p, segment_ttl(segments, c.read), self.compacted);
            Some(rebuild(c, dropped, cause))
        });
        let next = prefix_after(segments, c);
        self.calls.insert(c.msg_id.clone(), CallState {
            prompt: c.prompt, model: c.model.clone(), effort: c.effort.clone(), speed: c.speed.clone(),
            ts: c.ts, start_ts: c.start_ts, request: c.request.clone(), segments: next,
        });
        self.last = Some(c.msg_id.clone());
        if result.is_some() { self.compacted = false; }
        if self.compacted { self.called_since_compact = true; }
        result
    }
}

fn cause_for(c: &CallInput, p: &CallState, ttl_ms: i64, compacted: bool) -> CacheBreakCause {
    if compacted { return CacheBreakCause::Compaction; }
    if p.model != c.model { return CacheBreakCause::ModelSwitch { from: p.model.clone() }; }
    if let (Some(from), Some(to)) = (&p.effort, &c.effort) {
        if from != to { return CacheBreakCause::EffortChange { from: from.clone(), to: to.clone() }; }
    }
    if let (Some(from), Some(to)) = (&p.speed, &c.speed) {
        if (from == "fast") != (to == "fast") { return CacheBreakCause::FastMode { on: to == "fast" }; }
    }
    let requests = c.request.as_ref().zip(p.request.as_ref());
    if let Some((a, b)) = requests {
        if a.sign_in.zip(b.sign_in).is_some_and(|(a, b)| a != b) {
            return CacheBreakCause::RequestSettings { sign_in: true };
        }
        if a.betas.zip(b.betas).is_some_and(|(a, b)| a != b) {
            return CacheBreakCause::RequestSettings { sign_in: false };
        }
    }
    let gap_ms = c.start_ts.zip(p.start_ts).or_else(|| c.ts.zip(p.ts)).map_or(0, |(a, b)| a - b);
    if gap_ms > ttl_ms { return CacheBreakCause::Idle { gap_ms, ttl_ms }; }
    if let Some((a, b)) = requests {
        if a.shape.zip(b.shape).is_some_and(|(a, b)| a != b) { return CacheBreakCause::SystemOrTools; }
        if a.keep.zip(a.prev_len).is_some_and(|(keep, len)| keep < len) { return CacheBreakCause::EarlierMessages; }
        return CacheBreakCause::RequestUnchanged;
    }
    CacheBreakCause::Unknown
}

/// The token at the read boundary is the first missing token, hence strict `>`.
fn segment_ttl(segments: &[(u64, i64)], read: u64) -> i64 {
    segments.iter().find(|(end, _)| *end > read).or_else(|| segments.last())
        .map_or(FIVE_MINUTES, |(_, ttl)| *ttl)
}

fn append_segment(segments: &mut Vec<(u64, i64)>, end: u64, ttl: i64) {
    if let Some(last) = segments.last_mut().filter(|(_, old_ttl)| *old_ttl == ttl) { last.0 = end; }
    else { segments.push((end, ttl)); }
}

fn prefix_after(previous: &[(u64, i64)], c: &CallInput) -> Vec<(u64, i64)> {
    let mut segments = Vec::new();
    for &(end, ttl) in previous.iter().take_while(|(end, _)| *end <= c.read) {
        append_segment(&mut segments, end, ttl);
    }
    if c.read > segments.last().map_or(0, |(end, _)| *end) {
        append_segment(&mut segments, c.read, segment_ttl(previous, c.read - 1));
    }
    let mut end = c.read;
    for (amount, ttl) in [(c.tokens.cache_write_1h, HOUR), (c.tokens.cache_write_5m, FIVE_MINUTES)] {
        if amount > 0 { end += amount; append_segment(&mut segments, end, ttl); }
    }
    segments
}

fn rebuild(c: &CallInput, dropped: u64, cause: CacheBreakCause) -> CacheBreak {
    let writes = Tokens { cache_write_5m: c.tokens.cache_write_5m, cache_write_1h: c.tokens.cache_write_1h, ..Tokens::default() };
    let cost_usd = price(&c.model).map(|p| price_tokens(&p, &writes, c.speed.as_deref() == Some("fast"), c.prompt > LONG_PROMPT).total());
    CacheBreak { ts: c.ts, dropped, written: writes.cache_write_5m + writes.cache_write_1h, cost_usd, cause }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, time: i64, read: u64, write: u64, hour: bool) -> CallInput {
        let tokens = Tokens { cache_read: read, cache_write_5m: if hour { 0 } else { write }, cache_write_1h: if hour { write } else { 0 }, ..Tokens::default() };
        CallInput { msg_id: id.into(), parent: None, ts: Some(time), start_ts: None, model: "claude-opus-5-5".into(), effort: None, speed: None, prompt: tokens.input_total(), read, tokens, request: None }
    }

    #[test]
    fn legacy_four_causes_and_both_ttls() {
        for hour in [false, true] {
            for mode in ["idle", "compact", "model", "unknown"] {
                let mut t = CacheTracker::default();
                assert!(t.call(&call("a", 0, 0, 50_000, hour)).is_none());
                let mut b = call("b", if mode == "idle" { HOUR + 1 } else { 1 }, 0, 40_000, hour);
                if mode == "compact" { t.compacted(); }
                if mode == "model" { b.model = "another-model".into(); }
                let got = t.call(&b).unwrap();
                let expected = match mode {
                    "idle" => CacheBreakCause::Idle { gap_ms: HOUR + 1, ttl_ms: if hour { HOUR } else { FIVE_MINUTES } },
                    "compact" => CacheBreakCause::Compaction,
                    "model" => CacheBreakCause::ModelSwitch { from: "claude-opus-5-5".into() },
                    _ => CacheBreakCause::Unknown,
                };
                assert_eq!(got.cause, expected);
                assert_eq!((got.dropped, got.written), (50_000, 40_000));
                if mode == "model" { assert_eq!(got.cost_usd, None); }
                else { assert!((got.cost_usd.unwrap() - if hour { 0.32 } else { 0.20 }).abs() < 1e-9); }
            }
        }
    }

    #[test]
    fn legacy_duplicates_and_threshold() {
        let mut t = CacheTracker::default();
        let a = call("a", 0, 0, 50_000, true);
        assert!(t.call(&a).is_none());
        assert!(t.call(&a).is_none());
        assert!(t.call(&call("b", 1, 40_001, 9_999, true)).is_none());
        assert_eq!(t.call(&call("c", 2, 40_000, 10_000, true)).unwrap().dropped, 10_000);
    }
    fn facts() -> RequestFacts {
        RequestFacts { shape: Some(1), betas: Some(2), sign_in: Some(false), keep: Some(10), prev_len: Some(10) }
    }

    #[test]
    fn rewind_uses_ancestor_for_drop_and_idle() {
        for idle in [false, true] {
            let mut t = CacheTracker::default();
            t.call(&call("a", 0, 0, 50_000, true));
            t.call(&call("b", 100, 50_000, 90_000, true));
            let mut c = call("c", if idle { HOUR + 1 } else { 200 }, if idle { 20_000 } else { 50_000 }, 70_000, true);
            c.parent = Some("a".into());
            let got = t.call(&c);
            if idle {
                let got = got.unwrap();
                assert_eq!(got.dropped, 30_000);
                assert_eq!(got.cause, CacheBreakCause::Idle { gap_ms: HOUR + 1, ttl_ms: HOUR });
            } else { assert!(got.is_none(), "rewind's abandoned suffix is not lost cache"); }
        }
    }

    #[test]
    fn mixed_ttls_choose_the_first_lost_segment() {
        for (read, expected) in [(0, CacheBreakCause::Unknown), (50_000, CacheBreakCause::Idle { gap_ms: 600_000, ttl_ms: FIVE_MINUTES })] {
            let mut t = CacheTracker::default();
            t.call(&call("a", 0, 0, 50_000, true));
            t.call(&call("b", 1, 50_000, 20_000, false));
            assert_eq!(t.call(&call("c", 600_001, read, 30_000, true)).unwrap().cause, expected);
        }
        let mut t = CacheTracker::default();
        t.call(&call("a", 0, 0, 50_000, true));
        t.call(&call("b", 1, 50_000, 1_000, false));
        assert!(t.call(&call("c", 600_001, 50_000, 2_000, false)).is_none());
    }

    #[test]
    fn request_start_times_take_precedence_and_missing_times_fall_back() {
        for missing in [false, true] {
            let mut t = CacheTracker::default();
            let mut a = call("a", 0, 0, 50_000, false);
            let mut b = call("b", 100, 0, 50_000, false);
            a.start_ts = Some(-600_000);
            b.start_ts = if missing { None } else { Some(0) };
            t.call(&a);
            assert_eq!(t.call(&b).unwrap().cause, if missing { CacheBreakCause::Unknown } else { CacheBreakCause::Idle { gap_ms: 600_000, ttl_ms: FIVE_MINUTES } });
        }
    }

    #[test]
    fn new_causes_follow_priority_and_missing_facts_stay_unknown() {
        // Every case also enables the causes below it, exercising the priority order.
        for (mode, expected) in [(0, "Compaction"), (1, "ModelSwitch"), (2, "EffortChange"), (3, "FastMode"), (4, "RequestSettings { sign_in: true }"), (5, "RequestSettings { sign_in: false }"), (6, "Idle"), (7, "SystemOrTools"), (8, "EarlierMessages"), (9, "RequestUnchanged"), (10, "Unknown")] {
            let mut t = CacheTracker::default();
            let mut a = call("a", 0, 0, 50_000, false);
            let mut b = call("b", if mode <= 6 { HOUR } else { 1 }, 0, 50_000, false);
            a.effort = Some("high".into()); b.effort = Some(if mode <= 2 { "xhigh" } else { "high" }.into());
            a.speed = Some("standard".into()); b.speed = Some(if mode <= 3 { "fast" } else { "standard" }.into());
            a.request = Some(facts());
            let mut f = facts();
            if mode <= 4 { f.sign_in = Some(true); }
            if mode <= 5 { f.betas = Some(3); }
            if mode <= 7 { f.shape = Some(4); }
            if mode <= 8 { f.keep = Some(0); }
            b.request = if mode == 10 { None } else { Some(f) };
            if mode <= 1 { b.model = "other".into(); }
            t.call(&a);
            if mode == 0 { t.compacted(); }
            let got = t.call(&b).unwrap();
            assert!(format!("{:?}", got.cause).starts_with(expected), "mode {mode}: {:?}", got.cause);
        }
    }

    #[test]
    fn missing_effort_and_speed_do_not_mean_a_change() {
        let mut t = CacheTracker::default();
        t.call(&call("a", 0, 0, 50_000, true));
        let mut b = call("b", 1, 0, 50_000, true);
        b.effort = Some("high".into()); b.speed = Some("standard".into());
        assert_eq!(t.call(&b).unwrap().cause, CacheBreakCause::Unknown);
    }

    #[test]
    fn compact_survives_hits_and_pre_call_turns_but_not_a_later_turn() {
        for new_turn in [false, true] {
            let mut t = CacheTracker::default();
            t.call(&call("a", 0, 0, 50_000, true));
            t.compacted(); t.new_turn();
            let b = call("b", 1, 50_000, 1_000, true);
            assert!(t.call(&b).is_none());
            assert!(t.call(&b).is_none());
            if new_turn { t.new_turn(); }
            assert_eq!(t.call(&call("c", 2, 0, 50_000, true)).unwrap().cause, if new_turn { CacheBreakCause::Unknown } else { CacheBreakCause::Compaction });
            assert_eq!(t.call(&call("d", 3, 0, 50_000, true)).unwrap().cause, CacheBreakCause::Unknown);
        }
    }

    #[test]
    fn segments_truncate_merge_and_extend_unknown_prefixes() {
        let segments = [(50_000, HOUR), (70_000, FIVE_MINUTES)];
        assert_eq!(prefix_after(&segments, &call("a", 0, 30_000, 5_000, true)), vec![(35_000, HOUR)]);
        assert_eq!(prefix_after(&segments, &call("a", 0, 60_000, 5_000, false)), vec![(50_000, HOUR), (65_000, FIVE_MINUTES)]);
        assert_eq!(prefix_after(&segments, &call("a", 0, 80_000, 5_000, true)), vec![(50_000, HOUR), (80_000, FIVE_MINUTES), (85_000, HOUR)]);
        assert_eq!(prefix_after(&[], &call("a", 0, 20_000, 5_000, false)), vec![(25_000, FIVE_MINUTES)]);
        assert_eq!(segment_ttl(&segments, 50_000), FIVE_MINUTES);
        assert_eq!(segment_ttl(&segments, 49_999), HOUR);
    }

    #[test]
    fn exact_expiry_and_reversed_or_missing_timestamps_are_not_idle() {
        for time in [Some(FIVE_MINUTES), Some(-1), None] {
            let mut t = CacheTracker::default();
            t.call(&call("a", 0, 0, 50_000, false));
            let mut b = call("b", 0, 0, 50_000, false); b.ts = time;
            b.parent = Some("missing".into());
            assert_eq!(t.call(&b).unwrap().cause, CacheBreakCause::Unknown);
        }
    }

    #[test]
    fn compact_reset_and_duplicate_do_not_consume_pending_marker() {
        let mut t = CacheTracker::default();
        let a = call("a", 0, 0, 50_000, true);
        t.call(&a); t.compacted();
        t.call(&call("b", 1, 50_000, 1_000, true));
        t.compacted(); t.new_turn();
        assert!(t.call(&a).is_none());
        assert_eq!(t.call(&call("c", 2, 0, 50_000, true)).unwrap().cause, CacheBreakCause::Compaction);
    }

    #[test]
    fn writes_use_fast_multiplier_and_final_prompt_for_long_pricing() {
        for (model, fast, prompt, expected) in [
            ("claude-opus-5-5", true, 50_000, 0.4),
            ("claude-haiku-5-5", false, 100_000, 0.005),
            ("claude-haiku-5-5", false, 100_001, 0.025),
        ] {
            let mut t = CacheTracker::default();
            t.call(&call("a", 0, 0, 50_000, false));
            let mut b = call("b", 1, 0, 40_000, false);
            b.model = model.into(); b.speed = Some(if fast { "fast" } else { "standard" }.into());
            b.prompt = prompt;
            // Billed input can be larger than the final message prompt.
            b.tokens.input = 200_000;
            assert!((t.call(&b).unwrap().cost_usd.unwrap() - expected).abs() < 1e-9);
        }
    }

    #[test]
    fn missing_request_fields_do_not_invent_changes_and_fast_can_turn_off() {
        let mut t = CacheTracker::default();
        let mut a = call("a", 0, 0, 50_000, true);
        a.request = Some(RequestFacts::default());
        let mut b = call("b", 1, 0, 50_000, true); b.request = Some(facts());
        t.call(&a);
        assert_eq!(t.call(&b).unwrap().cause, CacheBreakCause::RequestUnchanged);
        let mut t = CacheTracker::default();
        a.speed = Some("fast".into()); b.speed = Some("standard".into());
        t.call(&a);
        assert_eq!(t.call(&b).unwrap().cause, CacheBreakCause::FastMode { on: false });
    }

}
