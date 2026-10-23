//! Behavior: the price table turns one API response's token usage into dollars the way the API
//! bills it. The two benchmarks are `cost-state.modelUsage` entries Claude Code wrote on the
//! reference machine (all cache writes there are 1-hour ones).

use agent_session_core::model::UsageRow;
use agent_session_core::pricing::{context_window, cost, normalize, price};

fn usage(model: &str) -> UsageRow {
    UsageRow {
        msg_id: "msg_1".into(),
        model: model.into(),
        ts: 0,
        input: 0,
        cache_write_5m: 0,
        cache_write_1h: 0,
        cache_read: 0,
        output: 0,
        web_search: 0,
        fast: false,
        cost_usd: None,
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn fable_5_1_matches_what_claude_code_charged() {
    let mut u = usage("claude-fable-5-1");
    (u.input, u.output, u.cache_read, u.cache_write_1h) = (360, 105_936, 15_443_177, 251_086);
    let c = cost(&u).unwrap();
    assert!(close(c, 14.182_914_25), "{c}");
}

#[test]
fn opus_5_with_the_1m_tag_matches_what_claude_code_charged() {
    let mut u = usage("claude-opus-5[1m]");
    (u.input, u.output, u.cache_read, u.cache_write_1h) = (128, 26_245, 4_777_934, 277_089);
    let c = cost(&u).unwrap();
    assert!(close(c, 5.816_622), "{c}");
}

#[test]
fn five_minute_cache_writes_cost_a_quarter_more_than_input() {
    let mut u = usage("claude-opus-5-5");
    u.cache_write_5m = 1_000_000;
    assert!(close(cost(&u).unwrap(), 5.0), "$4 input × 1.25");
    u.cache_write_5m = 0;
    u.cache_write_1h = 1_000_000;
    assert!(close(cost(&u).unwrap(), 8.0), "$4 input × 2");
}

#[test]
fn fast_mode_doubles_every_token_but_not_web_searches() {
    let mut u = usage("claude-opus-5-5");
    (u.input, u.output, u.cache_read, u.web_search) = (1_000_000, 1_000_000, 1_000_000, 3);
    let standard = cost(&u).unwrap();
    assert!(close(standard, 4.0 + 20.0 + 0.20 + 0.03));
    u.fast = true;
    assert!(close(cost(&u).unwrap(), (4.0 + 20.0 + 0.20) * 2.0 + 0.03));
}

#[test]
fn haiku_5_5_charges_five_times_more_once_the_prompt_passes_100k() {
    let mut u = usage("claude-haiku-5-5");
    (u.input, u.cache_read, u.output) = (10_000, 90_000, 1_000_000);
    assert_eq!(u.prompt_tokens(), 100_000);
    assert!(close(cost(&u).unwrap(), 0.001 + 0.0009 + 0.5), "exactly 100K is still the low tier");
    u.input += 1;
    let high = cost(&u).unwrap();
    assert!(close(high, (10_001.0 * 0.50 + 90_000.0 * 0.05 + 1_000_000.0 * 2.50) / 1e6), "{high}");
}

#[test]
fn model_ids_are_normalized_before_lookup() {
    assert_eq!(normalize("claude-opus-5[1m]"), "claude-opus-5");
    assert_eq!(normalize("claude-haiku-4-5-20251001"), "claude-haiku-4-5");
    assert_eq!(normalize("claude-opus-5-5"), "claude-opus-5-5");
    assert_eq!(normalize("claude-opus-4-1-20250805[1m]"), "claude-opus-4-1");
    let haiku = price("claude-haiku-4-5-20251001").unwrap();
    assert_eq!((haiku.input, haiku.output, haiku.cache_read), (1.0, 5.0, 0.10));
    let sonnet = price("claude-sonnet-5-5").unwrap();
    assert_eq!(sonnet.cache_read, 0.10, "Sonnet 5.5 cache hits are 5% of input");
    assert_eq!(price("claude-mythos-5-1").map(|p| p.cache_read), Some(0.25));
}

#[test]
fn unknown_models_have_no_price_and_no_window() {
    assert!(price("<synthetic>").is_none());
    assert!(cost(&usage("gpt-5")).is_none());
    assert_eq!(context_window("claude-opus-9"), None);
}

#[test]
fn context_windows_are_looked_up_per_model() {
    assert_eq!(context_window("claude-haiku-4-5-20251001"), Some(200_000));
    assert_eq!(context_window("claude-opus-5"), Some(1_000_000), "1M with or without the [1m] tag");
    assert_eq!(context_window("claude-opus-5[1m]"), Some(1_000_000));
    assert_eq!(context_window("claude-fable-5-1"), Some(1_000_000));
    assert_eq!(context_window("claude-haiku-5-5"), Some(1_000_000));
}
