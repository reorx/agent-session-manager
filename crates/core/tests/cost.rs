//! Behavior: a session's API usage, grouped by what decides its price, is re-priced as if its
//! main model had been another one. Only the main model's calls switch; the others keep the
//! price they had. The token counts stay the same: this is a price comparison, not a forecast
//! of how another model would have worked.

use agent_session_core::cost::{compare, primary_model};
use agent_session_core::model::{ModelUsage, Tokens};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn group(model: &str, tokens: Tokens) -> ModelUsage {
    ModelUsage { model: model.into(), fast: false, long_prompt: false, responses: 1, tokens, cost_usd: None }
}

/// One million of every kind of input token, and of output.
fn million() -> Tokens {
    Tokens { input: 1_000_000, cache_write_5m: 1_000_000, cache_write_1h: 1_000_000, cache_read: 1_000_000, output: 1_000_000, web_search: 0 }
}

fn output(n: u64) -> Tokens {
    Tokens { output: n, ..Tokens::default() }
}

#[test]
fn the_main_model_is_the_one_that_cost_the_most() {
    let haiku = ModelUsage { responses: 40, ..group("claude-haiku-4-5", output(1_000_000)) };
    let usage = [haiku.clone(), group("claude-opus-5-5", output(500_000))];
    assert_eq!(primary_model(&usage), Some("claude-opus-5-5"), "$10 of Opus beats $5 of Haiku, however many calls");
    let usage = [haiku, group("claude-opus-5-5", output(100_000))];
    assert_eq!(primary_model(&usage), Some("claude-haiku-4-5"), "$5 of Haiku beats $2 of Opus");
    assert_eq!(primary_model(&[]), None);
}

#[test]
fn opus_5_5_on_fable_5_2_doubles_everything_but_cache_reads() {
    let usage = [group("claude-opus-5-5", million())];
    let c = compare(&usage, "claude-fable-5-2").unwrap();
    assert_eq!((c.source.as_str(), c.target.as_str()), ("claude-opus-5-5", "claude-fable-5-2"));
    let actual = c.actual.unwrap();
    assert!(close(actual.input, 4.0) && close(c.predicted.input, 8.0));
    assert!(close(actual.cache_write_5m, 5.0) && close(c.predicted.cache_write_5m, 10.0));
    assert!(close(actual.cache_write_1h, 8.0) && close(c.predicted.cache_write_1h, 16.0));
    assert!(close(actual.cache_read, 0.20) && close(c.predicted.cache_read, 0.20), "same price for cache hits");
    assert!(close(actual.output, 20.0) && close(c.predicted.output, 40.0));
    assert!(close(c.actual_total().unwrap(), 37.20));
    assert!(close(c.predicted_total(), 74.20));
    assert_eq!(c.tokens, million());
}

#[test]
fn other_models_keep_their_price_on_both_sides() {
    let haiku = group("claude-haiku-4-5-20251001", output(1_000_000));
    let usage = [group("claude-opus-5-5", output(1_000_000)), haiku];
    let c = compare(&usage, "claude-sonnet-5-5").unwrap();
    assert_eq!(c.source, "claude-opus-5-5");
    assert_eq!(c.others.len(), 1);
    assert_eq!(c.others[0].model, "claude-haiku-4-5-20251001");
    assert!(close(c.others_usd, 5.0), "Haiku 4.5 output at $5");
    assert!(close(c.actual_total().unwrap(), 20.0 + 5.0));
    assert!(close(c.predicted_total(), 10.0 + 5.0), "only the Opus calls moved to Sonnet");
}

#[test]
fn groups_of_the_main_model_add_up_and_count_their_fast_and_long_calls() {
    let usage = [
        ModelUsage { fast: true, responses: 2, ..group("claude-opus-5-5", output(1_000_000)) },
        ModelUsage { long_prompt: true, responses: 3, ..group("claude-opus-5-5", output(1_000_000)) },
    ];
    let c = compare(&usage, "claude-fable-5-2").unwrap();
    assert_eq!((c.responses, c.fast_responses, c.long_prompt_responses), (5, 2, 3));
    assert_eq!(c.tokens.output, 2_000_000);
    assert!(close(c.actual.unwrap().output, 20.0 * 2.0 + 20.0), "fast calls at the fast price");
    assert!(close(c.predicted.output, 40.0 * 2.0 + 40.0), "Fable 5.2 has a fast mode too");
    let sonnet = compare(&usage, "claude-sonnet-5-5").unwrap();
    assert!(close(sonnet.predicted.output, 10.0 + 10.0), "no fast mode on Sonnet 5.5: the standard price");
    let haiku = compare(&usage, "claude-haiku-5-5").unwrap();
    assert!(close(haiku.predicted.output, 0.50 + 0.50 * 5.0), "the long-prompt calls cost ×5 on Haiku 5.5");
}

#[test]
fn an_unpriced_main_model_has_no_actual_cost_but_still_a_prediction() {
    let c = compare(&[group("claude-opus-9", output(1_000_000))], "claude-opus-5-5").unwrap();
    assert!(c.actual.is_none() && c.actual_total().is_none());
    assert!(close(c.predicted_total(), 20.0));
}

#[test]
fn nothing_to_compare_without_usage_or_with_an_unknown_target() {
    assert!(compare(&[], "claude-fable-5-2").is_none());
    assert!(compare(&[group("claude-opus-5-5", output(1))], "claude-opus-9").is_none());
}

#[test]
fn the_cache_hit_ratio_is_the_share_of_input_read_from_the_cache() {
    use agent_session_core::model::SessionSummary;
    let mut s = SessionSummary::transcript("s", "-p", std::path::Path::new("/p/s.jsonl"));
    assert_eq!(s.cache_hit_ratio(), None, "no input, nothing to hit");
    (s.tokens_in, s.tokens_cache_write, s.tokens_cache_read) = (10, 190, 800);
    assert!(close(s.cache_hit_ratio().unwrap(), 0.8), "reads over uncached + written + read");
    s.tokens_cache_read = 0;
    assert_eq!(s.cache_hit_ratio(), Some(0.0), "input that never hit the cache is 0, not missing");
    s.tokens_out = 1_000;
    assert_eq!(s.cache_hit_ratio(), Some(0.0), "output does not count");
}
