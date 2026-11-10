//! Behavior: the price table turns one API response's token usage into dollars the way the API
//! bills it. The two benchmarks are `cost-state.modelUsage` entries Claude Code wrote on the
//! reference machine (all cache writes there are 1-hour ones).

use agent_session_core::model::UsageRow;
use agent_session_core::claude::model_catalog::ModelCatalog;
use agent_session_core::pricing::{CURRENT_MODELS, EARLIER_MODELS, builtin_window, context_window, cost, display_name, normalize, price, price_tokens, resolve};

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
    assert_eq!(builtin_window("claude-opus-9"), None);
}

#[test]
fn context_windows_are_looked_up_per_model() {
    assert_eq!(builtin_window("claude-haiku-4-5-20251001"), Some(200_000));
    assert_eq!(builtin_window("claude-opus-5"), Some(1_000_000), "1M with or without the [1m] tag");
    assert_eq!(builtin_window("claude-opus-5[1m]"), Some(1_000_000));
    assert_eq!(builtin_window("claude-fable-5-1"), Some(1_000_000));
    assert_eq!(builtin_window("claude-haiku-5-5"), Some(1_000_000));
    for model in ["claude-haiku-4-5-20251001", "claude-opus-5", "claude-opus-5[1m]", "claude-fable-5-1", "claude-haiku-5-5", "claude-opus-9"] {
        assert_eq!(context_window(&ModelCatalog::default(), model), builtin_window(model));
    }
}

#[test]
fn fable_5_2_costs_twice_opus_5_5_except_for_cache_reads() {
    let fable = price("claude-fable-5-2").unwrap();
    assert_eq!((fable.input, fable.output, fable.cache_read), (8.0, 40.0, 0.20));
    assert_eq!(price("claude-mythos-5-2"), Some(fable), "Mythos 5.2 is priced like Fable 5.2");
    let mut u = usage("claude-fable-5-2");
    u.cache_write_5m = 1_000_000;
    assert!(close(cost(&u).unwrap(), 10.0), "5-minute writes: $8 × 1.25");
    u.cache_write_5m = 0;
    u.cache_write_1h = 1_000_000;
    assert!(close(cost(&u).unwrap(), 16.0), "1-hour writes: $8 × 2");
    u.fast = true;
    assert!(close(cost(&u).unwrap(), 32.0), "fast mode doubles it: $16 / $80 per MTok");
}

#[test]
fn a_price_splits_into_one_line_per_kind_of_token_that_add_up_to_the_cost() {
    let mut u = usage("claude-opus-5-5");
    (u.input, u.cache_write_5m, u.cache_write_1h, u.cache_read, u.output, u.web_search) = (1_000_000, 1_000_000, 1_000_000, 1_000_000, 1_000_000, 3);
    let lines = price_tokens(&price("claude-opus-5-5").unwrap(), &u.tokens(), false, false);
    assert!(close(lines.input, 4.0));
    assert!(close(lines.cache_write_5m, 5.0));
    assert!(close(lines.cache_write_1h, 8.0));
    assert!(close(lines.cache_read, 0.20));
    assert!(close(lines.output, 20.0));
    assert!(close(lines.web_search, 0.03));
    assert!(close(lines.input_total(), 4.0 + 5.0 + 8.0 + 0.20), "everything sent to the model");
    assert!(close(lines.total(), cost(&u).unwrap()));
    let long = price_tokens(&price("claude-haiku-5-5").unwrap(), &u.tokens(), false, true);
    let short = price_tokens(&price("claude-haiku-5-5").unwrap(), &u.tokens(), false, false);
    assert!(close(long.output, short.output * 5.0), "Haiku 5.5 prices long prompts ×5");
    assert!(close(long.web_search, short.web_search), "searches cost the same either way");
}

#[test]
fn model_ids_read_as_family_and_version() {
    assert_eq!(display_name("claude-opus-5-5"), "Opus 5.5");
    assert_eq!(display_name("claude-fable-5-2"), "Fable 5.2");
    assert_eq!(display_name("claude-fable-5"), "Fable 5");
    assert_eq!(display_name("claude-haiku-4-5-20251001"), "Haiku 4.5");
    assert_eq!(display_name("claude-opus-5[1m]"), "Opus 5");
    assert_eq!(display_name("claude-3-5-haiku"), "Haiku 3.5");
    assert_eq!(display_name("gpt-5"), "gpt-5", "anything else stays as it is");
}

#[test]
fn a_model_can_be_named_without_its_claude_prefix() {
    assert_eq!(resolve("fable-5-2"), Some("claude-fable-5-2".to_string()));
    assert_eq!(resolve("claude-opus-5-5"), Some("claude-opus-5-5".to_string()));
    assert_eq!(resolve("opus-9"), None, "only models with a price");
}

#[test]
fn every_comparison_model_has_a_price() {
    assert_eq!(CURRENT_MODELS, ["claude-fable-5-2", "claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"]);
    for model in CURRENT_MODELS.iter().chain(EARLIER_MODELS) {
        assert!(price(model).is_some(), "{model}");
    }
}
