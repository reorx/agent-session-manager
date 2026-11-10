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

/// `$3.42`, two decimals (sub-cent costs show as `$0.00`).
pub fn fmt_usd(usd: f64) -> String {
    format!("${usd:.2}")
}

/// `512`, `41k`, `1.2M`, `1M`, `12.7B` — token counts at a glance.
pub fn fmt_tokens(n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    let k = (n as f64 / 1000.0).round();
    if k < 1000.0 {
        return format!("{k:.0}k");
    }
    let (value, unit) = if n < 999_950_000 { (n as f64 / 1e6, "M") } else { (n as f64 / 1e9, "B") };
    let text = format!("{value:.1}");
    format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
}

/// How far `to` is from `from`: `+52%`, `-50%`, `0%`; a dash when there is nothing to compare.
pub fn fmt_change(from: f64, to: f64) -> String {
    if from <= 0.0 {
        return "—".into();
    }
    let pct = (to - from) / from * 100.0;
    if pct.abs() < 0.5 { "0%".into() } else { format!("{pct:+.0}%") }
}

/// Parse an RFC 3339 timestamp (`2026-09-01T08:00:01.000Z`) into unix ms.
pub fn parse_ts(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_counts_step_up_to_billions() {
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(41_200), "41k");
        assert_eq!(fmt_tokens(1_000_000), "1M");
        assert_eq!(fmt_tokens(13_140_000), "13.1M");
        assert_eq!(fmt_tokens(999_949_999), "999.9M");
        assert_eq!(fmt_tokens(999_950_000), "1B");
        assert_eq!(fmt_tokens(12_687_987_541), "12.7B");
    }

    #[test]
    fn a_change_is_a_signed_percentage() {
        assert_eq!(fmt_change(5288.80, 8040.0), "+52%");
        assert_eq!(fmt_change(10.0, 5.0), "-50%");
        assert_eq!(fmt_change(10.0, 10.01), "0%");
        assert_eq!(fmt_change(0.0, 5.0), "—");
    }
}
