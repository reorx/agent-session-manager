//! Compact record projection and deterministic replay. No transcript text is persisted here.
use std::{collections::HashMap, hash::{DefaultHasher, Hash, Hasher}};
use super::{cache::{CacheTracker, CallInput, RequestFacts}, parse::{Content, SYNTHETIC_MODEL, prompt_of, tokens_of, user_text}};
use crate::{format::parse_ts, model::*, transcript::{Record, RecordFilter, is_interruption, is_local_command_output, opens_turn}};

pub(crate) fn project(line: &[u8], byte_offset: u64) -> Option<CacheEvent> {
    // Use the Reader's schema and field-level fallback, including for otherwise skipped graph
    // nodes. RawValue borrows tool/blob payloads; no request tail or tool text is persisted.
    let r: Record = serde_json::from_slice(line).ok()?;
    let content = r.content();
    let user_input = r.is_user_input(content.as_ref());
    let ts = r.timestamp.as_deref().and_then(parse_ts);
    let kind = match r.kind.as_deref() {
        Some("api-request") => {
            let betas = r.params.as_ref().and_then(|p| p.betas.as_ref()).map(|items| {
                let mut items = items.clone(); items.sort_unstable(); items.dedup(); items
            });
            CacheEventKind::Request(CacheRequest {
                id: r.id.clone(), ts, query_source: r.query_source.clone(), shape: r.shape_hash.clone(), betas,
                keep: r.keep, len: r.keep.unwrap_or(0).saturating_add(r.tail.as_ref().map_or(0, |t| t.len() as u32)),
            })
        }
        Some("system") if r.subtype.as_deref() == Some("compact_boundary") => CacheEventKind::Compact { boundary: true },
        Some("user") if r.message.is_some() => user_event(&r, content.as_ref().unwrap_or(&Content::Other)),
        Some("assistant") => call_event(&r, ts).map_or(CacheEventKind::Node, CacheEventKind::Call),
        _ => CacheEventKind::Node,
    };
    // UUID-only records still affect duplicate filtering and ancestry, regardless of type.
    if r.uuid.is_none() && matches!(kind, CacheEventKind::Node) { return None; }
    Some(CacheEvent { byte_offset, uuid: r.uuid, parent_uuid: r.parent_uuid,
        user_input, sidechain: r.is_sidechain, kind })
}

fn user_event(r: &Record, content: &Content) -> CacheEventKind {
    if r.is_compact_summary { return CacheEventKind::Compact { boundary: false }; }
    if matches!(content, Content::Blocks(b) if b.iter().any(|b| b.kind.as_deref() == Some("tool_result"))) {
        return CacheEventKind::Node;
    }
    let text = user_text(content).unwrap_or_default();
    if is_interruption(&r.user_flags(), &text) { return CacheEventKind::Node; }
    if opens_turn(&r.user_flags(), content) { return CacheEventKind::Turn; }
    if r.origin.as_ref().and_then(|o| o.kind.as_deref()).is_some_and(|s| s != "human") {
        if !text.trim().is_empty() { return CacheEventKind::Auto; }
    } else if is_local_command_output(&text) { return CacheEventKind::Auto; }
    CacheEventKind::Node
}

fn call_event(r: &Record, ts: Option<i64>) -> Option<CacheCallFacts> {
    let m = r.message.as_ref()?;
    let id = m.id.as_ref()?;
    let model = m.model.as_deref().filter(|m| *m != SYNTHETIC_MODEL)?;
    let usage = m.usage.as_ref()?;
    let (prompt, read) = prompt_of(usage);
    Some(CacheCallFacts { msg_id: id.clone(), ts, model: model.to_owned(), effort: r.effort.clone(),
        speed: usage.speed.as_deref().map(str::to_owned), request_id: r.request_ref.clone(),
        tokens: tokens_of(usage), prompt, read })
}

#[derive(Default)]
struct Replay {
    filter: RecordFilter,
    tracker: CacheTracker,
    parents: HashMap<String, Option<String>>,
    call_of: HashMap<String, String>,
    requests: HashMap<String, (Option<i64>, RequestFacts)>,
    last_len: HashMap<Option<String>, u32>,
    number: u32,
    turn: Option<u32>,
    had_compact: bool,
    calls: Vec<IndexedCacheCall>,
    breaks: Vec<IndexedCacheBreak>,
}

pub(crate) fn replay(events: &[CacheEvent]) -> (Vec<IndexedCacheCall>, Vec<IndexedCacheBreak>) {
    let mut replay = Replay::default();
    for event in events { replay.event(event); }
    (replay.calls, replay.breaks)
}

impl Replay {
    fn event(&mut self, e: &CacheEvent) {
        if !self.filter.keep(e.uuid.as_deref(), e.sidechain, e.user_input) { return; }
        if let Some(uuid) = &e.uuid { self.parents.insert(uuid.clone(), e.parent_uuid.clone()); }
        match &e.kind {
            CacheEventKind::Node => {},
            CacheEventKind::Turn => {
                self.tracker.new_turn(); self.number += 1; self.turn = Some(self.number);
            }
            CacheEventKind::Auto => self.turn = None,
            CacheEventKind::Compact { boundary } => {
                self.tracker.compacted();
                if *boundary || !self.had_compact { self.turn = None; }
                self.had_compact = true;
            }
            CacheEventKind::Request(r) => self.request(r),
            CacheEventKind::Call(c) => self.call(e, c),
        }
    }

    fn request(&mut self, r: &CacheRequest) {
        let prev_len = self.last_len.insert(r.query_source.clone(), r.len);
        let Some(id) = &r.id else { return; };
        let facts = RequestFacts { shape: r.shape.as_ref().map(fingerprint),
            betas: r.betas.as_ref().map(fingerprint),
            sign_in: r.betas.as_ref().map(|b| b.iter().any(|b| b.starts_with("oauth-"))),
            keep: r.keep, prev_len };
        self.requests.insert(id.clone(), (r.ts, facts));
    }

    fn ancestor(&self, e: &CacheEvent, id: &str) -> Option<String> {
        let mut ancestor = e.parent_uuid.as_ref();
        for _ in 0..10_000 {
            let uuid = ancestor?;
            if let Some(candidate) = self.call_of.get(uuid).filter(|s| s.as_str() != id) { return Some(candidate.clone()); }
            ancestor = self.parents.get(uuid).and_then(Option::as_ref);
        }
        None
    }

    fn call(&mut self, e: &CacheEvent, c: &CacheCallFacts) {
        if let Some(uuid) = &e.uuid { self.call_of.insert(uuid.clone(), c.msg_id.clone()); }
        if self.tracker.contains(&c.msg_id) { return; }
        let parent = self.ancestor(e, &c.msg_id);
        let request = c.request_id.as_ref().and_then(|id| self.requests.get(id));
        let input = CallInput { msg_id: c.msg_id.clone(), parent: parent.clone(), ts: c.ts,
            start_ts: request.and_then(|(ts, _)| *ts), model: c.model.clone(), effort: c.effort.clone(), speed: c.speed.clone(),
            tokens: c.tokens.clone(), prompt: c.prompt, read: c.read, request: request.map(|(_, facts)| facts.clone()) };
        if let Some(rebuilt) = self.tracker.call(&input) {
            self.breaks.push(IndexedCacheBreak { msg_id: c.msg_id.clone(), byte_offset: e.byte_offset,
                turn: self.turn, model: c.model.clone(), rebuilt });
        }
        self.calls.push(IndexedCacheCall { byte_offset: e.byte_offset, turn: self.turn, parent_msg_id: parent, facts: c.clone() });
    }
}

fn fingerprint(value: &impl Hash) -> u64 {
    let mut hash = DefaultHasher::new(); value.hash(&mut hash); hash.finish()
}
