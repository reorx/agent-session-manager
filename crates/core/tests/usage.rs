mod common;
use agent_session_core::{usage::*, model::*, pricing};
use chrono::{FixedOffset, NaiveDate, TimeZone};
fn day(s: &str) -> NaiveDate { s.parse().unwrap() }
fn ms(s: &str) -> i64 { chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis() }

#[test]
fn the_usage_day_starts_at_four() {
    let tz = FixedOffset::east_opt(8 * 3600).unwrap();
    assert_eq!(usage_day(ms("2026-11-16T03:59:59+08:00"), &tz), day("2026-11-15"));
    assert_eq!(usage_day(ms("2026-11-16T04:00:00+08:00"), &tz), day("2026-11-16"));
}
#[test]
fn a_day_starts_at_four_local_in_whole_hour_and_half_hour_zones() {
    for offset in [8*3600, -5*3600, 19800] {
        let tz = FixedOffset::east_opt(offset).unwrap();
        let start = day_start_ms(day("2026-11-16"), &tz);
        assert_eq!(tz.timestamp_millis_opt(start).unwrap().naive_local(), day("2026-11-16").and_hms_opt(4,0,0).unwrap());
    }
}
#[test]
fn days_around_a_dst_change_are_23_and_25_hours_long() {
    let tz = chrono_tz::America::New_York;
    for (a,b,h) in [("2026-03-07","2026-03-08",23),("2026-10-31","2026-11-01",25)] {
        assert_eq!(day_start_ms(day(b), &tz)-day_start_ms(day(a), &tz), h*3600000);
    }
    // Subtract wall-clock hours, not four elapsed hours across the DST jump.
    assert_eq!(usage_day(ms("2026-03-08T04:00:00-04:00"), &tz), day("2026-03-08"));
}
fn session(id: &str, hour: i64, model: &str) -> SessionUsage {
    SessionUsage { session: LedgerSession { id: id.into(), ..Default::default() }, hours: vec![HourlyUsage {
        hour, model: model.into(), responses: 2, tokens: Tokens { input: 100, output: 200, cache_read: 300, ..Default::default() }, ..Default::default()
    }] }
}
#[test]
fn totals_cover_the_hours_in_range_and_nothing_else() {
    let mut data = UsageData::default();
    data.replace_all(vec![session("a",1,"claude-sonnet-5"),session("b",2,"claude-sonnet-5"),session("c",3,"claude-sonnet-5")]);
    assert_eq!(data.totals(3600000,10800000).responses,4);
    assert_eq!(data.totals(3600000,10800000).tokens.input,200);
}
#[test]
fn costs_follow_the_price_table() {
    let mut s = session("a",1,"claude-haiku-5-5");
    s.hours[0].fast = true;
    s.hours[0].long_prompt = true;
    let p = pricing::ModelPrice { input: 2., output: 8., cache_read: 0.2, window: 1000000, fast_mult: 6., long_prompt: Some(2.) };
    let mut data = UsageData::default();
    data.replace_all(vec![s.clone(), session("b",1,"other")]);
    let total = data.totals_with(0,7200000, |_| Some(p));
    let mut expected = pricing::price_tokens(&p,&s.hours[0].tokens,true,true);
    expected.add(&pricing::price_tokens(&p,&s.hours[0].tokens,false,false));
    assert_eq!(total.cost, expected);
    let doubled = data.totals_with(0,7200000, |m| Some(if m == "other" {p} else { pricing::ModelPrice {input:p.input*2.,output:p.output*2.,cache_read:p.cache_read*2.,..p} }));
    assert!((doubled.cost.total()-total.cost.total()-pricing::price_tokens(&p,&s.hours[0].tokens,true,true).total()).abs()<1e-10);
}
#[test]
fn a_model_without_a_price_costs_nothing_and_is_counted_as_unpriced() {
    let mut data=UsageData::default();
    data.apply(vec![session("a",1,"unknown")]);
    let t=data.totals(0,7200000);
    assert_eq!(t.unpriced_responses,2);
    assert_eq!(t.tokens.input,100);
    assert_eq!(t.cost.total(),0.);
}
#[test]
fn applying_changed_sessions_replaces_only_those_sessions() {
    let mut data=UsageData::default();
    data.replace_all(vec![session("a",1,"unknown"),session("b",1,"unknown")]);
    data.apply(vec![session("a",2,"unknown")]);
    assert_eq!(data.totals(3600000,7200000).responses,2);
    assert_eq!(data.totals(7200000,10800000).responses,2);
    data.replace_all(vec![]);
    assert_eq!(data.totals(0,i64::MAX).responses,0);
}
#[test]
fn hourly_costs_add_up_to_the_per_response_costs() {
    use agent_session_core::index::{Db, Indexer};
    let sb=common::sandbox();
    let mut db=Db::open(&sb.db_path()).unwrap();
    Indexer::new(sb.claude.clone(),None).scan(&mut db,&mut |_|{}).unwrap();
    let sessions=db.ledger_usage(None).unwrap().unwrap();
    let expected: f64=sessions.iter().flat_map(|s|db.usage(&s.session.id).unwrap()).filter_map(|r|pricing::cost(&r)).sum();
    let mut data=UsageData::default();
    data.replace_all(sessions);
    assert!((data.totals(0,i64::MAX).cost.total()-expected).abs()<1e-10);
}

#[test]
fn a_missing_local_four_uses_the_next_existing_instant() {
    let tz=chrono_tz::Pacific::Apia;
    // Samoa skipped the entire calendar day on its move across the date line.
    assert_eq!(day_start_ms(day("2011-12-30"),&tz),ms("2011-12-31T00:00:00+14:00"));
}
