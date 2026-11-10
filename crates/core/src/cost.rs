//! Cost analysis: what a session would have cost had its main model been another one. The
//! session's grouped usage (`SessionSummary::model_usage`) is re-priced with the same token
//! counts; only the main model's calls switch, every other model keeps its own price.

use crate::model::{ModelUsage, Tokens};
use crate::pricing::{CostLines, price, price_tokens};

/// A model of the session other than the main one, priced as it ran.
#[derive(Clone, Debug, PartialEq)]
pub struct OtherModel {
    pub model: String,
    pub responses: u64,
    /// `None` when the model has no price.
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    /// The main model (`primary_model`).
    pub source: String,
    pub target: String,
    /// Calls of the main model, and how many of them ran in fast mode or had a long prompt.
    pub responses: u64,
    pub fast_responses: u64,
    pub long_prompt_responses: u64,
    /// Tokens of the main model's calls.
    pub tokens: Tokens,
    /// The main model's calls at its own price; `None` when it has no price.
    pub actual: Option<CostLines>,
    /// The same calls at the target's price.
    pub predicted: CostLines,
    pub others: Vec<OtherModel>,
    /// What the other models cost, in both totals.
    pub others_usd: f64,
}

impl Comparison {
    pub fn actual_total(&self) -> Option<f64> {
        Some(self.actual?.total() + self.others_usd)
    }

    pub fn predicted_total(&self) -> f64 {
        self.predicted.total() + self.others_usd
    }
}

/// The model that cost the most (one without a price counts as free); a tie goes to the one
/// with more calls.
pub fn primary_model(usage: &[ModelUsage]) -> Option<&str> {
    let mut by_model: Vec<(&str, f64, u64)> = Vec::new();
    for group in usage {
        match by_model.iter_mut().find(|(model, ..)| *model == group.model) {
            Some((_, usd, calls)) => {
                *usd += group_cost(group).unwrap_or(0.0);
                *calls += group.responses;
            }
            None => by_model.push((&group.model, group_cost(group).unwrap_or(0.0), group.responses)),
        }
    }
    by_model.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(a.2.cmp(&b.2))).map(|(model, ..)| model)
}

/// The session priced as it ran next to the same calls with `target` as the main model.
/// `None` when the session made no API call or `target` has no price.
pub fn compare(usage: &[ModelUsage], target: &str) -> Option<Comparison> {
    let target_price = price(target)?;
    let source = primary_model(usage)?.to_string();
    let source_price = price(&source);
    let mut c = Comparison {
        source,
        target: target.to_string(),
        responses: 0,
        fast_responses: 0,
        long_prompt_responses: 0,
        tokens: Tokens::default(),
        actual: source_price.map(|_| CostLines::default()),
        predicted: CostLines::default(),
        others: Vec::new(),
        others_usd: 0.0,
    };
    for group in usage {
        if group.model != c.source {
            add_other(&mut c, group);
            continue;
        }
        c.responses += group.responses;
        c.fast_responses += if group.fast { group.responses } else { 0 };
        c.long_prompt_responses += if group.long_prompt { group.responses } else { 0 };
        c.tokens.add(&group.tokens);
        if let (Some(actual), Some(price)) = (&mut c.actual, &source_price) {
            actual.add(&price_tokens(price, &group.tokens, group.fast, group.long_prompt));
        }
        c.predicted.add(&price_tokens(&target_price, &group.tokens, group.fast, group.long_prompt));
    }
    Some(c)
}

/// A group at its own model's price, from its tokens (equal to the stored cost).
fn group_cost(group: &ModelUsage) -> Option<f64> {
    Some(price_tokens(&price(&group.model)?, &group.tokens, group.fast, group.long_prompt).total())
}

fn add_other(c: &mut Comparison, group: &ModelUsage) {
    let usd = group_cost(group);
    c.others_usd += usd.unwrap_or(0.0);
    match c.others.iter_mut().find(|o| o.model == group.model) {
        Some(other) => {
            other.responses += group.responses;
            if let Some(usd) = usd {
                *other.cost_usd.get_or_insert(0.0) += usd;
            }
        }
        None => c.others.push(OtherModel { model: group.model.clone(), responses: group.responses, cost_usd: usd }),
    }
}
