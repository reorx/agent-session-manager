//! List prices of Claude models, used to price every API response in a transcript.
//!
//! Checked against https://platform.claude.com/docs/en/about-claude/pricing on 2026-10-23;
//! Fable 5.2 / Mythos 5.2 added from Anthropic's model list on 2026-11-10; every row re-checked
//! on 2026-11-12 against the copy in `kb/refs/claude-api-pricing.md`.
//! Stored costs are computed at index time: after changing a price, bump
//! `index::db::SCHEMA_VERSION` so the index is rebuilt (about 4 seconds).
//! Context windows prefer Claude Code's cached catalog, then this table. They are resolved
//! at display time, so catalog changes do not require an index rebuild.
//!
//! Not modelled: US-only inference (`inference_geo`, ×1.1; never seen in local transcripts) and
//! the old 1M-context beta premium of Sonnet 4 / 4.5.

use crate::claude::model_catalog::ModelCatalog;
use crate::model::{Tokens, UsageRow};

/// Prices in USD per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    /// Context window in tokens.
    pub window: u64,
    /// Fast mode multiplies every token price (cache multipliers stack on top); 1 for a model
    /// without a fast mode.
    pub fast_mult: f64,
    /// Multiplier for prompts over `LONG_PROMPT` tokens: Haiku 5.5 prices them higher.
    pub long_prompt: Option<f64>,
}

impl ModelPrice {
    pub fn cache_write_5m(&self) -> f64 {
        self.input * CACHE_WRITE_5M
    }

    pub fn cache_write_1h(&self) -> f64 {
        self.input * CACHE_WRITE_1H
    }
}

/// Prompt size (every input token of a request) above which a long-prompt tier applies.
pub const LONG_PROMPT: u64 = 100_000;
const CACHE_WRITE_5M: f64 = 1.25;
const CACHE_WRITE_1H: f64 = 2.0;
const WEB_SEARCH_USD: f64 = 0.01;
const M: u64 = 1_000_000;
const K200: u64 = 200_000;

/// What a session can be re-priced as, current generation first (`cost::compare`).
pub const CURRENT_MODELS: [&str; 4] = ["claude-fable-5-2", "claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"];
pub const EARLIER_MODELS: &[&str] = &[
    "claude-fable-5-1",
    "claude-opus-5",
    "claude-sonnet-5",
    "claude-opus-4-8",
    "claude-sonnet-4-6",
    "claude-haiku-4-5",
];

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
        "claude-fable-5-2" | "claude-mythos-5-2" => fast(p(8.0, 40.0, 0.20, M)),
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
        "claude-haiku-5-5" => ModelPrice { long_prompt: Some(5.0), ..p(0.10, 0.50, 0.01, M) },
        "claude-haiku-4-5" => p(1.0, 5.0, 0.10, K200),
        "claude-3-5-haiku" => p(0.80, 4.0, 0.08, K200),
        _ => return None,
    })
}

/// A model id from a short name: `fable-5-2` → `claude-fable-5-2`. `None` without a price.
pub fn resolve(name: &str) -> Option<String> {
    let id = if name.starts_with("claude-") { name.to_string() } else { format!("claude-{name}") };
    price(&id).map(|_| id)
}

/// `claude-opus-5-5` → `Opus 5.5`, `claude-3-5-haiku` → `Haiku 3.5`; anything that is not a
/// Claude model id comes back as it is.
pub fn display_name(model: &str) -> String {
    let Some(rest) = normalize(model).strip_prefix("claude-") else { return model.to_string() };
    let (mut family, mut version) = (None, Vec::new());
    for part in rest.split('-') {
        match part.bytes().all(|b| b.is_ascii_digit()) {
            true => version.push(part),
            false if family.is_none() => family = Some(part),
            false => return model.to_string(),
        }
    }
    let Some(family) = family else { return model.to_string() };
    let mut chars = family.chars();
    let family: String = chars.next().map(|c| c.to_ascii_uppercase()).into_iter().chain(chars).collect();
    if version.is_empty() { family } else { format!("{family} {}", version.join(".")) }
}

/// USD for each kind of token.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CostLines {
    pub input: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
    pub output: f64,
    pub web_search: f64,
}

impl CostLines {
    /// What every input token cost: uncached, written to the cache, read from it.
    pub fn input_total(&self) -> f64 {
        self.input + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }

    pub fn total(&self) -> f64 {
        self.input_total() + self.output + self.web_search
    }

    pub fn add(&mut self, other: &CostLines) {
        self.input += other.input;
        self.cache_write_5m += other.cache_write_5m;
        self.cache_write_1h += other.cache_write_1h;
        self.cache_read += other.cache_read;
        self.output += other.output;
        self.web_search += other.web_search;
    }
}

/// `tokens` at `price`'s rates: in fast mode, and with prompts over `LONG_PROMPT`, when the
/// flags say so. Web searches cost the same either way.
pub fn price_tokens(price: &ModelPrice, tokens: &Tokens, fast: bool, long_prompt: bool) -> CostLines {
    let mut mult = if fast { price.fast_mult } else { 1.0 };
    if long_prompt && let Some(factor) = price.long_prompt {
        mult *= factor;
    }
    let usd = |n: u64, per_m: f64| n as f64 * per_m * mult / 1e6;
    CostLines {
        input: usd(tokens.input, price.input),
        cache_write_5m: usd(tokens.cache_write_5m, price.cache_write_5m()),
        cache_write_1h: usd(tokens.cache_write_1h, price.cache_write_1h()),
        cache_read: usd(tokens.cache_read, price.cache_read),
        output: usd(tokens.output, price.output),
        web_search: tokens.web_search as f64 * WEB_SEARCH_USD,
    }
}

/// What one API response cost, or `None` when the model has no known price.
pub fn cost(u: &UsageRow) -> Option<f64> {
    let price = price(&u.model)?;
    Some(price_tokens(&price, &u.tokens(), u.fast, u.prompt_tokens() > LONG_PROMPT).total())
}

pub fn builtin_window(model: &str) -> Option<u64> {
    price(model).map(|p| p.window)
}

pub fn context_window(catalog: &ModelCatalog, model: &str) -> Option<u64> {
    catalog.window(model).or_else(|| builtin_window(model))
}
