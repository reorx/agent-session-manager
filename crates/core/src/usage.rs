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

const HOUR: i64 = 3_600_000;
use chrono::{Datelike, Timelike};
use serde::{Serialize, Deserialize};
use crate::{model::LedgerSession, session::SessionKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UsageRange { #[default] Today, Yesterday, Days7, Days30, ThisMonth, LastMonth, Custom, Day }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Metric { #[default] Cost, In, Out, HitRate }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span { pub from_ms: i64, pub until_ms: i64 }
impl Span {
    fn contains_hour(self, hour: i64) -> bool {
        (self.from_ms.div_euclid(HOUR)..self.until_ms.div_euclid(HOUR)).contains(&hour)
    }
}

pub fn range_span<T: TimeZone>(range: UsageRange, today: NaiveDate, custom: Option<(NaiveDate,NaiveDate)>, day: Option<NaiveDate>, tz: &T) -> Span {
    let month=today.with_day(1).unwrap();
    let (a,b)=match range {
        UsageRange::Today => (today,today),
        UsageRange::Yesterday => (today-Duration::days(1),today-Duration::days(1)),
        UsageRange::Days7 => (today-Duration::days(6),today),
        UsageRange::Days30 => (today-Duration::days(29),today),
        UsageRange::ThisMonth => (month,today),
        UsageRange::LastMonth => ((month-Duration::days(1)).with_day(1).unwrap(),month-Duration::days(1)),
        UsageRange::Custom => custom.unwrap_or((month,today)),
        UsageRange::Day => (day.unwrap_or(today),day.unwrap_or(today)),
    };
    Span{from_ms:day_start_ms(a.min(b),tz),until_ms:day_start_ms(a.max(b)+Duration::days(1),tz)}
}

pub fn previous_span<T: TimeZone>(range: UsageRange, span: Span, now_ms: i64, tz: &T) -> Option<Span> {
    if span.until_ms<=span.from_ms { return None; }
    let first=usage_day(span.from_ms,tz);
    let end=usage_day(span.until_ms,tz);
    let days=(end-first).num_days();
    let previous_end=first;
    let previous_first=if matches!(range,UsageRange::ThisMonth|UsageRange::LastMonth) {
        (first.with_day(1).unwrap()-Duration::days(1)).with_day(1).unwrap()
    } else { first-Duration::days(days) };
    let today=usage_day(now_ms,tz);
    let partial=range!=UsageRange::Day && first<=today && today<end;
    let until_ms=if partial {
        let last = previous_first + Duration::days(days - 1);
        let now = tz.timestamp_millis_opt(now_ms).single().expect("valid timestamp").naive_local();
        let date = last + Duration::days(i64::from(now.hour() < DAY_START_HOUR));
        let mut local = date.and_time(now.time());
        // Round the wall clock before resolving DST. Missing hours advance to the next
        // existing instant; repeated hours use their earliest occurrence, like day_start_ms.
        if local.minute() != 0 || local.second() != 0 || local.nanosecond() != 0 {
            local = local.with_minute(0).unwrap().with_second(0).unwrap()
                .with_nanosecond(0).unwrap() + Duration::hours(1);
        }
        while tz.from_local_datetime(&local).earliest().is_none() {
            local += Duration::seconds(1);
        }
        tz.from_local_datetime(&local).earliest().unwrap().timestamp_millis()
            .min(day_start_ms(previous_end, tz))
    } else { day_start_ms(previous_end,tz) };
    Some(Span{from_ms:day_start_ms(previous_first,tz),until_ms})
}

pub fn hour_slots<T: TimeZone>(day: NaiveDate,tz: &T) -> Vec<Span> {
    let end=day_start_ms(day+Duration::days(1),tz);
    (day_start_ms(day,tz)..end).step_by(HOUR as usize).map(|start|Span{from_ms:start,until_ms:(start+HOUR).min(end)}).collect()
}
pub fn day_slots<T: TimeZone>(span: Span,tz: &T) -> Vec<(NaiveDate,Span)> {
    let mut result=Vec::new(); let mut day=usage_day(span.from_ms,tz);
    while day_start_ms(day,tz)<span.until_ms {
        let next=day+Duration::days(1);
        result.push((day,Span{from_ms:day_start_ms(day,tz).max(span.from_ms),until_ms:day_start_ms(next,tz).min(span.until_ms)})); day=next;
    }
    result
}
pub fn moving_average(values: &[f64],window: usize) -> Vec<f64> {
    if window==0 {return Vec::new();}
    let mut sum=0.; values.iter().enumerate().map(|(i,v)| {sum+=v; if i>=window {sum-=values[i-window];} sum/((i+1).min(window) as f64)}).collect()
}
pub fn model_slot(model: &str) -> usize {
    match pricing::normalize(model) { "claude-opus-5-5"=>0,"claude-sonnet-5-5"=>1,"claude-fable-5-2"=>2,"claude-haiku-5-5"=>3,_=>4 }
}
#[derive(Clone, Debug)]
pub struct UsageFilter { pub project: Option<String>, pub cli: bool, pub sdk: bool, pub bg: bool, pub model: Option<String> }
impl Default for UsageFilter { fn default()->Self {Self{project:None,cli:true,sdk:true,bg:true,model:None}} }
impl UsageFilter {
    fn accepts(&self,s: &LedgerSession)->bool {
        self.project.as_ref().is_none_or(|p|p==&s.project_key) && match SessionKind::from_metadata(s.is_background,s.entrypoint.as_deref()) {SessionKind::Cli=>self.cli,SessionKind::Sdk=>self.sdk,SessionKind::Background=>self.bg,SessionKind::History=>false}
    }
}
#[derive(Clone, Debug, Default)]
pub struct Summary {pub totals:Totals,pub by_model:Vec<(String,Totals)>,pub by_project:Vec<ProjectUsage>,pub by_session:Vec<SessionTotal>,pub sessions:usize,pub projects:usize}
#[derive(Clone, Debug)]
pub struct ProjectUsage {pub key:String,pub name:String,pub totals:Totals,pub sessions:usize}
#[derive(Clone, Debug)]
pub struct SessionTotal {pub id:String,pub title:String,pub project_name:String,pub totals:Totals}
#[derive(Clone, Debug, Default)]
pub struct SlotTotals {pub totals:Totals,pub by_model:[Totals;5]}
#[derive(Clone, Debug)]
pub struct ProjectEntry {pub key:String,pub name:String,pub sessions:usize}
impl Totals {
    pub fn add(&mut self,other:&Self) {self.responses+=other.responses;self.tokens.add(&other.tokens);self.cost.add(&other.cost);self.unpriced_responses+=other.unpriced_responses;}
}
fn project_name(s:&LedgerSession)->String {
    s.project_path.as_deref().and_then(|p|std::path::Path::new(p).file_name()).and_then(|p|p.to_str()).unwrap_or(&s.project_key).to_string()
}
impl UsageData {
    fn visit(&self,filter:&UsageFilter,mut visit:impl FnMut(&LedgerSession,&crate::model::HourlyUsage)) {
        for s in self.sessions.values().filter(|s|filter.accepts(&s.session)) {
            for h in &s.hours {
                if filter.model.as_deref().is_some_and(|m|pricing::normalize(m)!=pricing::normalize(&h.model)) {continue;}
                visit(&s.session,h);
            }
        }
    }
    pub fn summary(&self,span:Span,filter:&UsageFilter)->Summary {self.summary_with(span,filter,pricing::price)}
    pub fn summary_with(&self,span:Span,filter:&UsageFilter,price:impl Fn(&str)->Option<ModelPrice>)->Summary {
        let mut result=Summary::default(); let mut models=BTreeMap::<String,Totals>::new(); let mut projects=BTreeMap::<String,ProjectUsage>::new(); let mut sessions=BTreeMap::<String,SessionTotal>::new();
        self.visit(filter,|s,h| {
            if !span.contains_hour(h.hour) || h.responses==0 {return;}
            let t = hour_totals(h, &price);
            result.totals.add(&t); models.entry(pricing::normalize(&h.model).into()).or_default().add(&t);
            let fresh=!sessions.contains_key(&s.id);
            sessions.entry(s.id.clone()).or_insert_with(||SessionTotal{id:s.id.clone(),title:s.title.clone(),project_name:project_name(s),totals:Totals::default()}).totals.add(&t);
            let p=projects.entry(s.project_key.clone()).or_insert_with(||ProjectUsage{key:s.project_key.clone(),name:project_name(s),totals:Totals::default(),sessions:0}); p.totals.add(&t); p.sessions+=usize::from(fresh);
        });
        result.by_model=models.into_iter().collect();result.by_project=projects.into_values().collect();result.by_session=sessions.into_values().collect();
        result.by_model.sort_by(|a,b|b.1.cost.total().total_cmp(&a.1.cost.total()));
        result.by_project.sort_by(|a,b|b.totals.cost.total().total_cmp(&a.totals.cost.total()));
        result.by_session.sort_by(|a,b|b.totals.cost.total().total_cmp(&a.totals.cost.total()));
        result.sessions=result.by_session.len();result.projects=result.by_project.len();result
    }
    pub fn series(&self,slots:&[Span],filter:&UsageFilter)->Vec<SlotTotals> {self.series_with(slots,filter,pricing::price)}
    pub fn series_with(&self,slots:&[Span],filter:&UsageFilter,price:impl Fn(&str)->Option<ModelPrice>)->Vec<SlotTotals> {
        let mut result=vec![SlotTotals::default();slots.len()];
        if slots.is_empty() { return result; }
        // Standard chart slots are sorted and disjoint. Retain the original semantics
        // for callers supplying overlapping or unordered intervals.
        let ordered = slots.iter().all(|s| s.from_ms <= s.until_ms)
            && slots.windows(2).all(|w| w[0].until_ms <= w[1].from_ms);
        self.visit(filter, |_, h| {
            if ordered {
                let i = slots.partition_point(|s| s.until_ms.div_euclid(HOUR) <= h.hour);
                if slots.get(i).is_some_and(|s| s.contains_hour(h.hour)) {
                    let t = hour_totals(h, &price);
                    result[i].totals.add(&t);
                    result[i].by_model[model_slot(&h.model)].add(&t);
                }
            } else {
                let mut total = None;
                for (slot, out) in slots.iter().zip(&mut result) {
                    if slot.contains_hour(h.hour) {
                        let t = total.get_or_insert_with(|| hour_totals(h, &price));
                        out.totals.add(t);
                        out.by_model[model_slot(&h.model)].add(t);
                    }
                }
            }
        });
        result
    }
    pub fn project_entries(&self)->Vec<ProjectEntry> {
        let mut entries=BTreeMap::<String,ProjectEntry>::new();
        for s in self.sessions.values().filter(|s|s.hours.iter().any(|h|h.responses>0)) {entries.entry(s.session.project_key.clone()).or_insert_with(||ProjectEntry{key:s.session.project_key.clone(),name:project_name(&s.session),sessions:0}).sessions+=1;}
        entries.into_values().collect()
    }
    pub fn model_entries(&self)->Vec<(String,u64)> {
        let mut entries=BTreeMap::new(); for h in self.sessions.values().flat_map(|s|&s.hours).filter(|h|h.responses>0) {*entries.entry(pricing::normalize(&h.model).to_string()).or_default()+=h.responses;} entries.into_iter().collect()
    }
}

fn hour_totals(h: &crate::model::HourlyUsage, price: &impl Fn(&str) -> Option<ModelPrice>) -> Totals {
    let p = price(&h.model);
    Totals {
        responses: h.responses,
        tokens: h.tokens,
        cost: p.map(|p| pricing::price_tokens(&p, &h.tokens, h.fast, h.long_prompt)).unwrap_or_default(),
        unpriced_responses: if p.is_none() { h.responses } else { 0 },
    }
}
