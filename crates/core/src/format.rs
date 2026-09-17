//! Human-readable formatting shared by the CLI and the UI, matching the Python script's output.

use chrono::{Local, TimeZone};

/// `185B`, `11.9K`, `1.2M`, `1.0G` — the reference script's `human_size`.
pub fn human_size(bytes: u64) -> String {
    let mut n = bytes as f64;
    for unit in ["B", "K", "M", "G"] {
        if n < 1024.0 || unit == "G" {
            return if unit == "B" { format!("{n:.0}{unit}") } else { format!("{n:.1}{unit}") };
        }
        n /= 1024.0;
    }
    unreachable!()
}

/// Local `YYYY-MM-DD HH:MM`, or `?` when there is no timestamp.
pub fn fmt_datetime(ms: Option<i64>) -> String {
    match ms.and_then(|ms| Local.timestamp_millis_opt(ms).single()) {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => "?".to_string(),
    }
}

/// Local `MM-DD HH:MM`, or `?` when there is no timestamp.
pub fn fmt_time(ms: Option<i64>) -> String {
    match ms.and_then(|ms| Local.timestamp_millis_opt(ms).single()) {
        Some(dt) => dt.format("%m-%d %H:%M").to_string(),
        None => "?".to_string(),
    }
}

/// Parse an RFC 3339 timestamp (`2026-09-01T08:00:01.000Z`) into unix ms.
pub fn parse_ts(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.timestamp_millis())
}
