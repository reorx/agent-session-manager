//! Local usage days and in-memory totals, priced with the current price table.
use std::collections::BTreeMap;
use chrono::{Duration, NaiveDate, TimeZone};
use crate::{model::{SessionUsage, Tokens}, pricing::{self, CostLines, ModelPrice}};

pub const DAY_START_HOUR: u32 = 4;

pub fn usage_day<Tz: TimeZone>(ts_ms: i64, tz: &Tz) -> NaiveDate {
    (tz.timestamp_millis_opt(ts_ms).single().expect("valid timestamp").naive_local()
        - Duration::hours(DAY_START_HOUR as i64)).date()
}

pub fn day_start_ms<Tz: TimeZone>(day: NaiveDate, tz: &Tz) -> i64 {
    let mut local = day.and_hms_opt(DAY_START_HOUR, 0, 0).unwrap();
    loop {
        if let Some(ts) = tz.from_local_datetime(&local).earliest() {
            return ts.timestamp_millis();
        }
        local += Duration::seconds(1);
    }
}

#[derive(Clone, Debug, Default)]
pub struct UsageData {
    pub sessions: BTreeMap<String, SessionUsage>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    pub responses: u64,
    pub tokens: Tokens,
    pub cost: CostLines,
    pub unpriced_responses: u64,
}

impl UsageData {
    pub fn replace_all(&mut self, sessions: Vec<SessionUsage>) {
        self.sessions.clear();
        self.apply(sessions);
    }

    pub fn apply(&mut self, sessions: Vec<SessionUsage>) {
        for session in sessions {
            self.sessions.insert(session.session.id.clone(), session);
        }
    }

    pub fn totals(&self, from_ms: i64, until_ms: i64) -> Totals {
        self.totals_with(from_ms, until_ms, pricing::price)
    }

    pub fn totals_with(&self, from_ms: i64, until_ms: i64, price: impl Fn(&str) -> Option<ModelPrice>) -> Totals {
        let mut total = Totals::default();
        for hour in self.sessions.values().flat_map(|s| &s.hours) {
            if !(from_ms / 3_600_000..until_ms / 3_600_000).contains(&hour.hour) { continue; }
            total.responses += hour.responses;
            total.tokens.add(&hour.tokens);
            if let Some(p) = price(&hour.model) {
                total.cost.add(&pricing::price_tokens(&p, &hour.tokens, hour.fast, hour.long_prompt));
            } else {
                total.unpriced_responses += hour.responses;
            }
        }
        total
    }
}
