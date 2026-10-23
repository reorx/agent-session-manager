//! List prices of Claude models, used to price every API response in a transcript.
//!
//! Checked against https://platform.claude.com/docs/en/about-claude/pricing on 2026-10-23.
//! Stored costs are computed at index time: after changing a price, bump
//! `index::db::SCHEMA_VERSION` so the index is rebuilt (about 4 seconds).
//!
//! Not modelled: US-only inference (`inference_geo`, ×1.1; never seen in local transcripts) and
//! the old 1M-context beta premium of Sonnet 4 / 4.5.

use crate::model::UsageRow;

/// Prices in USD per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    /// Context window in tokens.
    pub window: u64,
    /// Fast mode multiplies every token price (cache multipliers stack on top).
    pub fast_mult: f64,
    /// (prompt tokens above which, multiplier): Haiku 5.5 prices prompts over 100K higher.
    pub long_prompt: Option<(u64, f64)>,
}

const CACHE_WRITE_5M: f64 = 1.25;
const CACHE_WRITE_1H: f64 = 2.0;
const WEB_SEARCH_USD: f64 = 0.01;
const M: u64 = 1_000_000;
const K200: u64 = 200_000;

const fn p(input: f64, output: f64, cache_read: f64, window: u64) -> ModelPrice {
    ModelPrice { input, output, cache_read, window, fast_mult: 1.0, long_prompt: None }
}

const fn fast(price: ModelPrice) -> ModelPrice {
    ModelPrice { fast_mult: 2.0, ..price }
}

/// `claude-opus-5[1m]` → `claude-opus-5`, `claude-haiku-4-5-20251001` → `claude-haiku-4-5`.
pub fn normalize(model: &str) -> &str {
    let model = model.strip_suffix("[1m]").unwrap_or(model);
    match model.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => model,
    }
}

pub fn price(model: &str) -> Option<ModelPrice> {
    Some(match normalize(model) {
        "claude-fable-5-1" | "claude-mythos-5-1" => p(10.0, 50.0, 0.25, M),
        "claude-fable-5" | "claude-mythos-5" => p(10.0, 50.0, 1.0, M),
        "claude-opus-5-5" => fast(p(4.0, 20.0, 0.20, M)),
        "claude-opus-5" | "claude-opus-4-8" => fast(p(5.0, 25.0, 0.50, M)),
        "claude-opus-4-7" | "claude-opus-4-6" => p(5.0, 25.0, 0.50, M),
        "claude-opus-4-5" => p(5.0, 25.0, 0.50, K200),
        "claude-opus-4-1" | "claude-opus-4" => p(15.0, 75.0, 1.50, K200),
        "claude-sonnet-5-5" => p(2.0, 10.0, 0.10, M),
        "claude-sonnet-5" => p(2.0, 10.0, 0.20, M),
        "claude-sonnet-4-6" => p(3.0, 15.0, 0.30, M),
        "claude-sonnet-4-5" | "claude-sonnet-4" => p(3.0, 15.0, 0.30, K200),
        "claude-haiku-5-5" => ModelPrice { long_prompt: Some((100_000, 5.0)), ..p(0.10, 0.50, 0.01, M) },
        "claude-haiku-4-5" => p(1.0, 5.0, 0.10, K200),
        "claude-3-5-haiku" => p(0.80, 4.0, 0.08, K200),
        _ => return None,
    })
}

/// What one API response cost, or `None` when the model has no known price.
pub fn cost(u: &UsageRow) -> Option<f64> {
    let price = price(&u.model)?;
    let mut mult = if u.fast { price.fast_mult } else { 1.0 };
    if let Some((threshold, factor)) = price.long_prompt
        && u.prompt_tokens() > threshold
    {
        mult *= factor;
    }
    let tokens = u.input as f64 * price.input
        + u.cache_write_5m as f64 * price.input * CACHE_WRITE_5M
        + u.cache_write_1h as f64 * price.input * CACHE_WRITE_1H
        + u.cache_read as f64 * price.cache_read
        + u.output as f64 * price.output;
    Some(tokens * mult / 1e6 + u.web_search as f64 * WEB_SEARCH_USD)
}

pub fn context_window(model: &str) -> Option<u64> {
    price(model).map(|p| p.window)
}
