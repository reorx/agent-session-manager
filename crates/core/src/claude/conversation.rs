//! Direct transcript reading for the detail panel. No index, subprocesses or UI dependencies.
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

use anyhow::{Context, Result};
use memchr::memchr_iter;
use serde_json::{Map, Value};

use super::cache::{CacheTracker, CallInput, RequestFacts};
use super::clean::clean_prompt;
use super::parse::{Content, RawBlock, SYNTHETIC_MODEL, map_file, parse_content, prompt_of, tokens_of, user_text};
use crate::format::parse_ts;
use crate::model::*;
use crate::search::{Highlighted, Matcher, regex::{ToolLine, ToolLineKind, stringify}};
use crate::transcript::{Record, RecordFilter, is_interruption, is_local_command_output, opens_turn};

pub const ENTRY_CHARS: usize = 20_000;
pub const RESULT_CHARS: usize = 3_000;
pub const INPUT_FIELD_CHARS: usize = 2_000;
pub const HIT_LEAD_CHARS: usize = 200;

/// History contains prompts only; preserve every row, including commands and empty prompts.
pub fn conversation_from_history(prompts: &[HistoryPrompt], matcher: Option<&Matcher>) -> Conversation {
    let items = prompts.iter().enumerate().map(|(i, p)| ConvItem::Turn(Turn {
        number: Some(i as u32 + 1), origin: None, ts: Some(p.ts),
        prompt: Marked::new(&p.text, ENTRY_CHARS, matcher),
        attachments: Vec::new(), steps: Vec::new(), reply: None,
        end: TurnEnd::NoReply, duration_ms: None, last_ts: Some(p.ts), cache_breaks: Vec::new(),
    })).collect();
    Conversation { history_only: true, items, bytes: 0, skipped_lines: 0 }
}

impl Marked {
    pub fn new(text: &str, cap: usize, matcher: Option<&Matcher>) -> Self {
        let chars = text.chars().count();
        let lines = text.lines().count();
        let initial_end = text.char_indices().nth(cap).map_or(text.len(), |(i, _)| i);
        let first = matcher.and_then(|m| m.re().find_iter(text).find(|m| !m.is_empty())).map(|m| m.start());
        let start = first.filter(|&i| i >= initial_end).map_or(0, |i| {
            text[..i].char_indices().rev().nth(HIT_LEAD_CHARS - 1).map_or(0, |(i, _)| i)
        });
        let end = text[start..].char_indices().nth(cap).map_or(text.len(), |(i, _)| start + i);
        let mut shown = String::new();
        if start > 0 { shown.push('…'); }
        shown.push_str(&text[start..end]);
        if end < text.len() { shown.push('…'); }
        let hits = matcher.map_or_else(Vec::new, |m| m.ranges(&shown));
        Self { text: shown, hits, chars, lines, kept: text[start..end].chars().count(), moved: start > 0 }
    }
}

impl Conversation {
    pub fn hit_places(&self) -> Vec<HitPlace> {
        let mut hits = Vec::new();
        for (item, card) in self.items.iter().enumerate() {
            let mut add = |part, yes| { if yes { hits.push(HitPlace { item, part }); } };
            match card {
                ConvItem::Compact(c) => add(Part::Summary, c.summary.as_ref().is_some_and(|m| !m.hits.is_empty())),
                ConvItem::Turn(t) => {
                    add(Part::Prompt, !t.prompt.hits.is_empty());
                    for (i, step) in t.steps.iter().enumerate() {
                        let yes = match &step.kind {
                            StepKind::Text(m) | StepKind::Thinking(m) => !m.hits.is_empty(),
                            StepKind::Tool(t) => t.input.iter().any(|l| !l.text.ranges.is_empty())
                                || t.result.as_ref().is_some_and(|r| !r.output.hits.is_empty()),
                        };
                        add(Part::Step(i), yes);
                    }
                    add(Part::Reply, t.reply.as_ref().is_some_and(|r| !r.text.hits.is_empty()));
                }
            }
        }
        hits
    }
}

pub fn tool_input_lines(input: &Map<String, Value>, matcher: Option<&Matcher>) -> Vec<ToolLine> {
    let ordered = ["description", "command"].into_iter().filter_map(|key| input.get(key).map(|v| (key, v)))
        .chain(input.iter().filter(|(k, _)| !matches!(k.as_str(), "description" | "command")).map(|(k, v)| (k.as_str(), v)));
    ordered.map(|(key, value)| {
        let m = Marked::new(&stringify(value), INPUT_FIELD_CHARS, matcher);
        let kind = match key {
            "description" => ToolLineKind::Description,
            "command" => ToolLineKind::Command,
            _ => ToolLineKind::Field(key.to_owned()),
        };
        ToolLine { kind, text: Highlighted { text: m.text, ranges: m.hits } }
    }).collect()
}

pub fn tool_arg(name: &str, input: &Map<String, Value>) -> String {
    let key = match name {
        "Bash" => "command",
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "file_path",
        "Grep" | "Glob" => "pattern",
        "WebFetch" => "url", "WebSearch" => "query",
        "Task" | "Agent" => "description", "Skill" => "skill",
        _ => "",
    };
    let value = input.get(key).and_then(Value::as_str)
        .or_else(|| input.values().find_map(Value::as_str)).unwrap_or("");
    let mut arg = value.lines().next().unwrap_or("").to_owned();
    if name == "Grep" {
        if let Some(path) = input.get("path").and_then(Value::as_str) { arg.push(' '); arg.push_str(path); }
    }
    arg.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect()
}

#[derive(Clone, PartialEq, Eq)]
enum MessageKey { Id(String), Record(usize) }

struct Candidate {
    start: usize,
    id: MessageKey,
    text: String,
    ts: Option<i64>,
}

struct Reader<'a> {
    conv: Conversation,
    current: Option<usize>,
    number: u32,
    record_no: usize,
    candidate: Option<Candidate>,
    tool_messages: HashSet<String>,
    calls: HashMap<String, (usize, usize)>,
    matcher: Option<&'a Matcher>,
    cache: CacheTracker,
    parents: HashMap<String, Option<String>>,
    call_of: HashMap<String, String>,
    requests: HashMap<String, (Option<i64>, RequestFacts)>,
    last_len: HashMap<Option<String>, u32>,
}

impl Reader<'_> {
    fn turn(&mut self) -> &mut Turn {
        let i = self.current.expect("current turn");
        let ConvItem::Turn(t) = &mut self.conv.items[i] else { unreachable!() }; t
    }

    fn start(&mut self, number: Option<u32>, origin: Option<String>, ts: Option<i64>, prompt: Marked, attachments: Vec<Attachment>) {
        self.finish();
        self.current = Some(self.conv.items.len());
        self.conv.items.push(ConvItem::Turn(Turn {
            number, origin, ts, prompt, attachments, steps: Vec::new(), reply: None,
            end: TurnEnd::NoReply, duration_ms: None, last_ts: ts, cache_breaks: Vec::new(),
        }));
    }

    fn ensure_turn(&mut self, ts: Option<i64>) {
        if self.current.is_none() { self.start(None, Some("session start".into()), ts, Marked::default(), Vec::new()); }
        if ts.is_some() { self.turn().last_ts = ts; }
    }

    fn finish(&mut self) {
        if self.current.is_none() { return; }
        if let Some(c) = self.candidate.take() {
            let text = Marked::new(&c.text, ENTRY_CHARS, self.matcher);
            let t = self.turn();
            if !matches!(t.end, TurnEnd::Interrupted { .. } | TurnEnd::ApiError { .. }) {
                t.steps.truncate(c.start);
                t.reply = Some(Reply { ts: c.ts, text });
                t.end = TurnEnd::Completed;
            }
        }
        self.current = None;
        self.tool_messages.clear();
    }

    fn record(&mut self, r: &Record, content: Content) {
        self.record_no += 1;
        let ts = r.timestamp.as_deref().and_then(parse_ts);
        if let Some(uuid) = &r.uuid { self.parents.insert(uuid.clone(), r.parent_uuid.clone()); }
        if r.kind.as_deref() == Some("api-request") {
            self.request(r, ts);
            return;
        }
        if r.kind.as_deref() == Some("system") {
            match r.subtype.as_deref() {
                Some("compact_boundary") => {
                    self.cache.compacted();
                    self.finish(); self.conv.items.push(ConvItem::Compact(Compaction { ts, summary: None }));
                }
                Some("turn_duration") if self.current.is_some() => {
                    let t = self.turn();
                    if let Some(ms) = r.duration_ms { t.duration_ms = Some(t.duration_ms.unwrap_or(0) + ms); }
                    if ts.is_some() { t.last_ts = ts; }
                }
                _ => {}
            }
            return;
        }
        if r.message.is_none() { return; }
        match r.kind.as_deref() {
            Some("user") => self.user(r, &content, ts),
            Some("assistant") => self.assistant(r, &content, ts),
            _ => {}
        }
    }

    fn user(&mut self, r: &Record, content: &Content, ts: Option<i64>) {
        let text = user_text(content).unwrap_or_default();
        if r.is_compact_summary {
            self.cache.compacted();
            let summary = Some(Marked::new(&text, ENTRY_CHARS, self.matcher));
            if let Some(ConvItem::Compact(c)) = self.conv.items.iter_mut().rev().find(|i| matches!(i, ConvItem::Compact(_))) {
                c.summary = summary;
            } else {
                self.finish(); self.conv.items.push(ConvItem::Compact(Compaction { ts, summary }));
            }
            return;
        }
        if let Content::Blocks(blocks) = content {
            if blocks.iter().any(|b| b.kind.as_deref() == Some("tool_result")) {
                for block in blocks.iter().filter(|b| b.kind.as_deref() == Some("tool_result")) { self.result(block, ts); }
                return;
            }
        }
        let flags = r.user_flags();
        if is_interruption(&flags, &text) {
            self.ensure_turn(ts); self.candidate = None; self.turn().end = TurnEnd::Interrupted { ts }; return;
        }
        if opens_turn(&flags, content) {
            self.cache.new_turn();
            self.number += 1;
            self.start(Some(self.number), None, ts, Marked::new(&clean_prompt(&text), ENTRY_CHARS, self.matcher), attachments(content));
        } else if let Some(origin) = r.origin.as_ref().and_then(|o| o.kind.as_deref()).filter(|k| *k != "human") {
            if !text.trim().is_empty() {
                self.start(None, Some(origin.to_owned()), ts, Marked::new(&text, ENTRY_CHARS, self.matcher), attachments(content));
            }
        } else if is_local_command_output(&text) {
            // part of the command's turn, but still shown (and searched) on a card of its own
            self.start(None, Some("command output".to_owned()), ts, Marked::new(&text, ENTRY_CHARS, self.matcher), Vec::new());
        }
    }

    fn result(&mut self, block: &RawBlock, ts: Option<i64>) {
        let text = block.content.map(parse_content).and_then(|c| user_text(&c)).unwrap_or_default();
        let output = Marked::new(&text, RESULT_CHARS, self.matcher);
        let result = ToolResult { bytes: text.len(), lines: output.lines, output, is_error: block.is_error.unwrap_or(false) };
        if let Some((item, step)) = block.tool_use_id.as_deref().and_then(|id| self.calls.remove(id)) {
            let ConvItem::Turn(t) = &mut self.conv.items[item] else { unreachable!() };
            let StepKind::Tool(call) = &mut t.steps[step].kind else { unreachable!() };
            call.result = Some(result);
            if ts.is_some() { t.last_ts = ts; }
        } else {
            self.ensure_turn(ts); self.candidate = None;
            self.turn().steps.push(Step { ts, kind: StepKind::Tool(ToolCall { name: "?".into(), arg: String::new(), input: Vec::new(), result: Some(result) }) });
        }
    }

    fn assistant(&mut self, r: &Record, content: &Content, ts: Option<i64>) {
        let message = r.message.as_ref().unwrap();
        self.track_cache(r, ts);
        if r.is_api_error || r.is_aborted {
            self.ensure_turn(ts); self.candidate = None;
            self.turn().end = if r.is_api_error {
                TurnEnd::ApiError { ts, text: Marked::new(&user_text(content).unwrap_or_default(), ENTRY_CHARS, self.matcher).text }
            } else { TurnEnd::Interrupted { ts } };
            return;
        }
        let has_tools = message.stop_reason.as_deref() == Some("tool_use") || matches!(content, Content::Blocks(b) if b.iter().any(|b| b.kind.as_deref() == Some("tool_use")));
        if has_tools {
            self.candidate = None;
            if let Some(id) = &message.id { self.tool_messages.insert(id.clone()); }
        }
        let key = message.id.as_ref().map_or(MessageKey::Record(self.record_no), |id| MessageKey::Id(id.clone()));
        let pure = !has_tools && !message.id.as_ref().is_some_and(|id| self.tool_messages.contains(id));
        match content {
            Content::Text(text) => self.text(text, &key, pure, ts),
            Content::Blocks(blocks) => for block in blocks {
                match block.kind.as_deref() {
                    Some("text") => self.text(block.text.as_deref().unwrap_or(""), &key, pure, ts),
                    Some("thinking") => {
                        let text = block.thinking.as_deref().unwrap_or("");
                        if text.trim().is_empty() { continue; }
                        self.ensure_turn(ts); self.candidate = None;
                        let marked = Marked::new(text, ENTRY_CHARS, self.matcher);
                        self.turn().end = TurnEnd::NoReply;
                        self.turn().steps.push(Step { ts, kind: StepKind::Thinking(marked) });
                    }
                    Some("tool_use") => self.call(block, ts),
                    _ => {}
                }
            },
            Content::Other => {}
        }
    }

    fn request(&mut self, r: &Record, ts: Option<i64>) {
        let prev_len = self.last_len.insert(r.query_source.clone(), r.keep.unwrap_or(0)
            .saturating_add(r.tail.as_ref().map_or(0, |tail| tail.len() as u32)));
        let Some(id) = &r.id else { return; };
        let betas = r.params.as_ref().and_then(|p| p.betas.as_ref()).map(|betas| {
            let mut sorted: Vec<_> = betas.iter().collect();
            sorted.sort_unstable(); sorted.dedup();
            sorted
        });
        let facts = RequestFacts {
            shape: r.shape_hash.as_ref().map(fingerprint),
            betas: betas.as_ref().map(fingerprint),
            sign_in: betas.as_ref().map(|items| items.iter().any(|b| b.starts_with("oauth-"))),
            keep: r.keep, prev_len,
        };
        self.requests.insert(id.clone(), (ts, facts));
    }

    fn ancestor_call(&self, r: &Record, id: &str) -> Option<String> {
        let mut ancestor = r.parent_uuid.as_ref();
        for _ in 0..10_000 {
            let uuid = ancestor?;
            if let Some(candidate) = self.call_of.get(uuid).filter(|candidate| candidate.as_str() != id) {
                return Some(candidate.clone());
            }
            ancestor = self.parents.get(uuid).and_then(Option::as_ref);
        }
        None
    }

    fn track_cache(&mut self, r: &Record, ts: Option<i64>) {
        let message = r.message.as_ref().unwrap();
        let (Some(id), Some(model), Some(usage)) = (&message.id, message.model.as_deref(), &message.usage) else { return; };
        if model == SYNTHETIC_MODEL { return; }
        // Every block's uuid participates in ancestry, even when its response was already seen.
        if let Some(uuid) = &r.uuid { self.call_of.insert(uuid.clone(), id.clone()); }
        if self.cache.contains(id) { return; }
        let parent = self.ancestor_call(r, id);
        let request = r.request_ref.as_ref().and_then(|id| self.requests.get(id));
        let (prompt, read) = prompt_of(usage);
        let call = CallInput {
            msg_id: id.clone(), parent, ts, start_ts: request.and_then(|(ts, _)| *ts),
            model: model.to_owned(), effort: r.effort.clone(), speed: usage.speed.as_deref().map(str::to_owned),
            tokens: tokens_of(usage), prompt, read, request: request.map(|(_, facts)| facts.clone()),
        };
        if let Some(rebuilt) = self.cache.call(&call) {
            self.ensure_turn(ts);
            self.turn().cache_breaks.push(rebuilt);
        }
    }

    fn text(&mut self, text: &str, id: &MessageKey, pure: bool, ts: Option<i64>) {
        let text = text.trim();
        if text.is_empty() { return; }
        self.ensure_turn(ts);
        self.turn().end = TurnEnd::NoReply;
        if pure {
            let same = self.candidate.as_ref().is_some_and(|c| &c.id == id);
            if same {
                let c = self.candidate.as_mut().unwrap(); c.text.push_str("\n\n"); c.text.push_str(text); c.ts = ts;
            } else {
                self.candidate = Some(Candidate { start: self.turn().steps.len(), id: id.clone(), text: text.to_owned(), ts });
            }
        } else { self.candidate = None; }
        let marked = Marked::new(text, ENTRY_CHARS, self.matcher);
        self.turn().steps.push(Step { ts, kind: StepKind::Text(marked) });
    }

    fn call(&mut self, block: &RawBlock, ts: Option<i64>) {
        self.ensure_turn(ts); self.candidate = None; self.turn().end = TurnEnd::NoReply;
        let input: Map<String, Value> = block.input.and_then(|raw| serde_json::from_str(raw.get()).ok()).unwrap_or_default();
        let name = block.name.as_deref().unwrap_or("?").to_owned();
        let call = ToolCall { arg: tool_arg(&name, &input), input: tool_input_lines(&input, self.matcher), name, result: None };
        let step = self.turn().steps.len();
        if let Some(id) = &block.id { self.calls.insert(id.to_string(), (self.current.unwrap(), step)); }
        self.turn().steps.push(Step { ts, kind: StepKind::Tool(call) });
    }
}

fn fingerprint(value: &impl Hash) -> u64 {
    let mut hash = DefaultHasher::new();
    value.hash(&mut hash);
    hash.finish()
}

fn attachments(content: &Content) -> Vec<Attachment> {
    let Content::Blocks(blocks) = content else { return Vec::new() };
    blocks.iter().filter_map(|b| {
        let kind = b.kind.as_deref().filter(|k| matches!(*k, "image" | "document"))?;
        // Base64 stays borrowed, including its JSON quotes; no decoding/allocation of image data.
        let len = b.source.as_ref().and_then(|s| s.data).map_or(0, |v| v.get().len().saturating_sub(2));
        Some(Attachment { kind: kind.into(), bytes: len as u64 * 3 / 4 })
    }).collect()
}

/// Read complete JSONL lines only. Invalid complete records are counted; an unfinished tail waits.
pub fn read_conversation(path: &Path, matcher: Option<&Matcher>) -> Result<Conversation> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let bytes = map_file(&file, file.metadata()?.len())?;
    let mut reader = Reader {
        conv: Conversation { history_only: false, items: Vec::new(), bytes: bytes.len() as u64, skipped_lines: 0 },
        current: None, number: 0, record_no: 0, candidate: None, tool_messages: HashSet::new(), calls: HashMap::new(), matcher,
        cache: CacheTracker::default(), parents: HashMap::new(), call_of: HashMap::new(),
        requests: HashMap::new(), last_len: HashMap::new(),
    };
    let mut filter = RecordFilter::for_file(path);
    let mut start = 0;
    for end in memchr_iter(b'\n', &bytes) {
        let line = &bytes[start..end]; start = end + 1;
        if line.iter().all(u8::is_ascii_whitespace) { continue; }
        match serde_json::from_slice::<Record>(line) {
            Ok(r) => {
                let content = r.content();
                if filter.keep(r.uuid.as_deref(), r.is_sidechain, r.is_user_input(content.as_ref())) {
                    reader.record(&r, content.unwrap_or(Content::Other));
                }
            }
            Err(_) => reader.conv.skipped_lines += 1,
        }
    }
    reader.finish();
    Ok(reader.conv)
}
