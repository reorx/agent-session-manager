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

#[test]
fn ranges_slots_and_previous_periods() {
    let tz=FixedOffset::east_opt(28800).unwrap();
    let today=day("2026-11-16");
    for (r,a,b) in [(UsageRange::Today,"2026-11-16","2026-11-17"),(UsageRange::Yesterday,"2026-11-15","2026-11-16"),(UsageRange::Days7,"2026-11-10","2026-11-17"),(UsageRange::Days30,"2026-10-18","2026-11-17"),(UsageRange::ThisMonth,"2026-11-01","2026-11-17"),(UsageRange::LastMonth,"2026-10-01","2026-11-01"),(UsageRange::Custom,"2026-11-01","2026-11-17"),(UsageRange::Day,"2026-11-16","2026-11-17")] {
        let s=range_span(r,today,None,None,&tz);
        assert_eq!(s,Span{from_ms:day_start_ms(day(a),&tz),until_ms:day_start_ms(day(b),&tz)});
        let p=previous_span(r,s,ms("2026-11-16T15:53:00+08:00"),&tz).unwrap();
        assert!(p.until_ms<=s.from_ms);
    }
    let s=range_span(UsageRange::Today,today,None,None,&tz);
    assert_eq!(previous_span(UsageRange::Today,s,ms("2026-11-16T15:53:00+08:00"),&tz).unwrap().until_ms,ms("2026-11-15T16:00:00+08:00"));
    let custom=range_span(UsageRange::Custom,today,Some((day("2026-11-01"),day("2026-11-03"))),None,&tz);
    assert_eq!(day_slots(custom,&tz).len(),3);
    assert_eq!(previous_span(UsageRange::Custom,custom,ms("2026-11-16T15:53:00+08:00"),&tz).unwrap().until_ms,custom.from_ms);
    assert_eq!(hour_slots(today,&tz).len(),24);
    for (d,n) in [("2026-03-07",23),("2026-10-31",25)] { assert_eq!(hour_slots(day(d),&chrono_tz::America::New_York).len(),n); }
}

#[test]
fn summary_filters_ranking_series_and_entries() {
    let mut a=session("a",1,"claude-opus-5-5");
    a.session.project_key="p".into(); a.session.project_path=Some("/gone/project".into()); a.session.title="Kept title".into();
    a.hours.push(session("x",2,"claude-haiku-5-5").hours.remove(0));
    let mut b=session("b",2,"unknown"); b.session.project_key="q".into(); b.session.entrypoint=Some("sdk-cli".into());
    let mut c=session("c",2,"claude-sonnet-5-5"); c.session.is_background=true;
    let mut data=UsageData::default(); data.replace_all(vec![a,b,c]);
    let span=Span{from_ms:7200000,until_ms:10800000}; let f=UsageFilter::default();
    let s=data.summary(span,&f);
    assert_eq!(s.sessions,3); assert_eq!(s.projects,3); assert_eq!(s.totals.responses,6); assert_eq!(s.totals.unpriced_responses,2);
    assert_eq!(s.by_session[0].id,"c");
    assert!((s.by_model.iter().map(|(_,t)|t.cost.total()).sum::<f64>()-s.totals.cost.total()).abs()<1e-10);
    assert!((s.by_project.iter().map(|p|p.totals.cost.total()).sum::<f64>()-s.totals.cost.total()).abs()<1e-10);
    assert_eq!(s.by_session.iter().find(|s|s.id=="a").unwrap().title,"Kept title");
    for filter in [UsageFilter{project:Some("p".into()),..f.clone()},UsageFilter{sdk:false,bg:false,..f.clone()},UsageFilter{model:Some("claude-haiku-5-5[1m]".into()),..f.clone()}] { assert_eq!(data.summary(span,&filter).totals.responses,2); }
    let slots=data.series(&[span],&f); assert_eq!(slots[0].totals,s.totals);
    assert_eq!(slots[0].by_model.iter().map(|t|t.responses).sum::<u64>(),6);
    assert_eq!(data.project_entries().len(),3); assert_eq!(data.model_entries().len(),4);
    for (m,i) in ["claude-opus-5-5","claude-sonnet-5-5","claude-fable-5-2","claude-haiku-5-5","unknown"].iter().zip(0..) {assert_eq!(model_slot(m),i);}
    assert_eq!(moving_average(&[1.,2.,3.,4.,5.,6.,7.,8.],7),vec![1.,1.5,2.,2.5,3.,3.5,4.,5.]);
}

#[test]
fn each_previous_range_has_exact_calendar_boundaries() {
    let tz=FixedOffset::east_opt(28800).unwrap(); let today=day("2026-11-16");
    for (r,a,b) in [(UsageRange::Today,"2026-11-15T04:00:00+08:00","2026-11-15T16:00:00+08:00"),(UsageRange::Yesterday,"2026-11-14T04:00:00+08:00","2026-11-15T04:00:00+08:00"),(UsageRange::Days7,"2026-11-03T04:00:00+08:00","2026-11-09T16:00:00+08:00"),(UsageRange::Days30,"2026-09-18T04:00:00+08:00","2026-10-17T16:00:00+08:00"),(UsageRange::ThisMonth,"2026-10-01T04:00:00+08:00","2026-10-16T16:00:00+08:00"),(UsageRange::LastMonth,"2026-09-01T04:00:00+08:00","2026-10-01T04:00:00+08:00"),(UsageRange::Day,"2026-11-15T04:00:00+08:00","2026-11-16T04:00:00+08:00")] {
        assert_eq!(previous_span(r,range_span(r,today,None,None,&tz),ms("2026-11-16T15:53:00+08:00"),&tz),Some(Span{from_ms:ms(a),until_ms:ms(b)}));
    }
}
#[test]
fn injected_prices_and_half_hour_buckets_are_consistent() {
    let mut data=UsageData::default(); data.apply(vec![session("a",1,"unknown")]);
    let s=Span{from_ms:5400000,until_ms:9000000};
    let p=pricing::price("claude-opus-5-5").unwrap();
    let t=data.summary_with(s,&UsageFilter::default(),|_|Some(p)).totals;
    assert_eq!(t.responses,2); assert_eq!(t.unpriced_responses,0);
    assert_eq!(t.cost,pricing::price_tokens(&p,&session("a",1,"x").hours[0].tokens,false,false));
    assert_eq!(t.tokens.input_total(),400);
    assert_eq!(data.series_with(&[s],&UsageFilter::default(),|_|Some(p))[0].totals,t);
}
#[test]
#[ignore = "explicit ledger COPY only: ASM_USAGE_BENCH_LEDGER"]
fn real_ledger_aggregation_timings() {
    let path=std::env::var("ASM_USAGE_BENCH_LEDGER").unwrap();
    let ledger=agent_session_core::ledger::Ledger::open(std::path::Path::new(&path)).unwrap();
    let mut data=UsageData::default(); data.replace_all(ledger.usage(None).unwrap());
    let tz=FixedOffset::east_opt(28800).unwrap();
    let span=range_span(UsageRange::Days30,day("2026-11-16"),None,None,&tz);
    let slots:Vec<_>=day_slots(span,&tz).into_iter().map(|(_,s)|s).collect();
    let start=std::time::Instant::now();
    for _ in 0..1000 {std::hint::black_box(data.summary(span,&UsageFilter::default()));std::hint::black_box(data.series(&slots,&UsageFilter::default()));}
    println!("sessions={} buckets={} summary+30day_series_mean_ms={:.3}",data.sessions.len(),data.sessions.values().map(|s|s.hours.len()).sum::<usize>(),start.elapsed().as_secs_f64());
}
