//! The scan side of the session search: read the transcripts of the sessions searched, in
//! parallel, skipping files and lines the byte prefilter rules out.
//!
//! Its items are the index's rows where the kinds overlap: the user records the indexer gives a
//! row (same line selection, same record filter), split into `prompt` and `other` by the same
//! turn rule, and an assistant record's text and thinking as the indexer joins them. Turn numbers
//! count the prompts up to an item the same way, so they match the index's.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::{Condvar, Mutex};

use aho_corasick::AhoCorasick;
use memchr::{memchr, memchr_iter, memrchr};
use memmap2::MmapMut;
use rayon::prelude::*;

use super::prefilter::Prefilter;
use super::query::{Query, indexes};
use super::regex::{Block, iter_texts};
use super::sessions::{
    Hit, ItemRef, Kind, KindSet, SNIPPET_AFTER, SNIPPET_BEFORE, SearchOptions, SessionHits, pick, tally, tool_input_text,
};
use crate::claude::parse::{Content, LineKind, RawRecord, classify, may_be_user_row, parse_content};
use crate::format::parse_ts;
use crate::model::SessionSummary;
use crate::transcript::{RecordFilter, has_own_input, opens_turn};

/// What a scan read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanStats {
    /// Transcripts opened.
    pub files: usize,
    /// Transcripts skipped whole because a TERM was missing from their bytes.
    pub skipped_files: usize,
    pub bytes: u64,
    pub skipped_bytes: u64,
}

/// Default `SearchOptions::read_budget`: 256 MiB. Without it, the threads each read one of the
/// biggest transcripts at once and a scan's peak grew with them (about 490 MB over 30 days on
/// 2026-11-15, decision 19).
pub const READ_BUDGET: u64 = 256 << 20;

/// Bytes of transcripts held in memory at once, shared by the scan's threads: a thread waits
/// until its file fits, and a file bigger than the whole budget waits for all of it.
struct Budget {
    total: u64,
    left: Mutex<u64>,
    freed: Condvar,
}

/// Bytes taken from the budget, given back when dropped.
struct Lease<'a> {
    budget: &'a Budget,
    bytes: u64,
}

impl Budget {
    fn new(total: u64) -> Budget {
        let total = total.max(1);
        Budget { total, left: Mutex::new(total), freed: Condvar::new() }
    }

    fn take(&self, bytes: u64) -> Lease<'_> {
        let bytes = bytes.min(self.total);
        let mut left = self.left.lock().expect("budget lock");
        while *left < bytes {
            left = self.freed.wait(left).expect("budget lock");
        }
        *left -= bytes;
        Lease { budget: self, bytes }
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        *self.budget.left.lock().expect("budget lock") += self.bytes;
        self.budget.freed.notify_all();
    }
}

struct Ctx<'q> {
    query: &'q Query,
    kinds: &'q KindSet,
    limit: usize,
    prefilter: Option<Prefilter>,
    budget: Budget,
    /// Kinds that come from user records (prompt, other, tool-output) / assistant records.
    user_side: bool,
    assistant_side: bool,
    tools: bool,
}

pub(crate) fn search_scan(sessions: &[SessionSummary], query: &Query, kinds: &KindSet, opts: &SearchOptions) -> (Vec<SessionHits>, ScanStats) {
    let has = |k| kinds.contains(k);
    let ctx = Ctx {
        query,
        kinds,
        limit: opts.snippets,
        prefilter: opts.prefilter.then(|| Prefilter::new(query, has(Kind::ToolInput))),
        budget: Budget::new(opts.read_budget),
        user_side: has(Kind::Prompt) || has(Kind::Other) || has(Kind::ToolOutput),
        assistant_side: has(Kind::Reply) || has(Kind::Thinking) || has(Kind::ToolInput),
        tools: has(Kind::ToolInput) || has(Kind::ToolOutput),
    };
    let mut files: Vec<(&SessionSummary, &Path)> =
        sessions.iter().filter(|s| !s.history_only()).filter_map(|s| Some((s, s.file_path.as_deref()?))).collect();
    // big files first, so that one of them does not start last and keep the scan waiting
    files.sort_by_key(|(s, _)| std::cmp::Reverse(s.file_size.unwrap_or(0)));
    let outcomes: Vec<FileOutcome> = files.par_iter().map(|(s, path)| scan_file(s, path, &ctx)).collect();
    let mut stats = ScanStats::default();
    let mut results = Vec::new();
    for o in outcomes {
        if o.opened {
            stats.files += 1;
            stats.bytes += o.bytes;
        }
        if o.skipped {
            stats.skipped_files += 1;
            stats.skipped_bytes += o.bytes;
        }
        results.extend(o.hits);
    }
    (results, stats)
}

#[derive(Default)]
struct FileOutcome {
    opened: bool,
    skipped: bool,
    bytes: u64,
    hits: Option<SessionHits>,
}

/// The tool an item belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Tool {
    None,
    Name(String),
    /// A tool result: the id of its call, whose name is looked up only when needed.
    Call(Option<String>),
}

/// One item with a TERM.
struct Found {
    item: ItemRef,
    ts: Option<i64>,
    turn: u32,
    tool: Tool,
}

fn scan_file(s: &SessionSummary, path: &Path, ctx: &Ctx) -> FileOutcome {
    let mut out = FileOutcome::default();
    let Ok(meta) = std::fs::metadata(path) else { return out };
    // held until the file's hits are made; released when this function returns
    let _lease = ctx.budget.take(meta.len());
    let Ok(bytes) = read_file(path, meta.len()) else { return out };
    out.opened = true;
    out.bytes = bytes.len() as u64;
    if ctx.prefilter.as_ref().is_some_and(|p| !p.file_may_match(&bytes)) {
        out.skipped = true;
        return out;
    }
    let (mut found, turns) = find_items(&bytes, path, ctx);
    if found.is_empty() {
        return out;
    }
    if !ctx.kinds.tools().is_empty() {
        name_tools(&bytes, found.iter_mut().collect());
        found.retain(|f| match &f.tool {
            Tool::Call(_) => false,
            Tool::Name(name) if f.item.kind == Kind::ToolOutput => ctx.kinds.tool_matches(name),
            _ => true,
        });
    }
    let items: Vec<ItemRef> = found.iter().map(|f| f.item).collect();
    let Some((counts, term_hits)) = tally(&items, ctx.query, ctx.kinds) else { return out };
    let picks = pick(&items, ctx.limit);
    let mut picked: Vec<Found> = Vec::with_capacity(picks.len());
    let mut found: Vec<Option<Found>> = found.into_iter().map(Some).collect();
    for i in picks {
        picked.push(found[i].take().expect("picked once"));
    }
    name_tools(&bytes, picked.iter_mut().collect());
    let snippets = picked.into_iter().map(|f| snippet(&bytes, f, ctx)).collect();
    out.hits = Some(SessionHits { session_id: s.id.clone(), updated_at: s.updated_at, counts, term_hits, turns, snippets });
    out
}

/// The first `len` bytes of the file (what it held when its size was taken; a line written since
/// is incomplete anyway).
///
/// Read, not mapped from the file: page faults on a mapped transcript cost several times a read
/// here (2026-11-15: 7 s vs 2 s over 30 days of transcripts). Read into an anonymous map rather
/// than a `Vec`: macOS's allocator keeps freed buffers this big for reuse, and with them a scan's
/// peak footprint reached about 500 MB however few files were in memory at once; the map is
/// given back to the system when dropped (2026-11-16 measurements in the P1-3 session note).
fn read_file(path: &Path, len: u64) -> std::io::Result<MmapMut> {
    let mut buf = MmapMut::map_anon(len as usize)?;
    File::open(path)?.read_exact(&mut buf)?;
    Ok(buf)
}

/// Every item with a TERM, in file order, with the turn it belongs to; and the turns in the file.
fn find_items(bytes: &[u8], path: &Path, ctx: &Ctx) -> (Vec<Found>, u32) {
    let mut found = Vec::new();
    let mut filter = RecordFilter::for_file(path);
    let mut turn: u32 = 0;
    for (offset, line) in lines(bytes) {
        let class = classify(line);
        // the lines the indexer parses for user rows: these decide the turns
        let turn_line = may_be_user_row(class, line);
        let wanted = match class {
            LineKind::Skip | LineKind::Meta => false,
            LineKind::User => ctx.user_side,
            LineKind::Assistant => ctx.assistant_side,
            LineKind::Unknown => true,
        };
        let term_line = wanted && ctx.prefilter.as_ref().is_none_or(|p| p.line_may_match(line));
        if !turn_line && !term_line {
            continue;
        }
        let Ok(rec) = serde_json::from_slice::<RawRecord>(line) else { continue };
        let mut user_kind = None;
        if turn_line && rec.kind.as_deref() == Some("user")
            && let Some(content) = rec.message.as_ref().and_then(|m| m.content).map(parse_content)
            && has_own_input(&content)
        {
            let kept = filter.keep(rec.uuid.as_deref(), rec.is_sidechain, true);
            user_kind = Some(if kept && opens_turn(&rec.user_flags(), &content) {
                turn += 1;
                Kind::Prompt
            } else {
                Kind::Other
            });
        }
        if !term_line {
            continue;
        }
        let ts = rec.timestamp.as_deref().and_then(parse_ts);
        for item in items(&rec, user_kind, ctx) {
            let mask = ctx.query.mask(&item.text);
            if mask != 0 {
                found.push(Found { item: ItemRef { kind: item.kind, offset, sub: item.sub, mask }, ts, turn, tool: item.tool });
            }
        }
    }
    (found, turn)
}

/// Complete lines (a partial last line is still being written, the indexer waits for it too)
/// with their offsets, without a trailing `\r`.
fn lines(bytes: &[u8]) -> impl Iterator<Item = (u64, &[u8])> {
    let mut start = 0;
    memchr_iter(b'\n', bytes).filter_map(move |nl| {
        let offset = start;
        let mut line = &bytes[start..nl];
        start = nl + 1;
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        (!line.is_empty()).then_some((offset as u64, line))
    })
}

struct Item<'a> {
    kind: Kind,
    sub: u32,
    tool: Tool,
    text: Cow<'a, str>,
}

/// The items of one record in the kinds searched. `user_kind` is the kind of a user record's
/// text, `None` when the indexer gives it no row.
fn items<'a>(rec: &RawRecord<'a>, user_kind: Option<Kind>, ctx: &Ctx) -> Vec<Item<'a>> {
    let kinds = ctx.kinds;
    let mut out = Vec::new();
    let pieces = iter_texts(rec, ctx.tools);
    match rec.kind.as_deref() {
        Some("user") => {
            // injected for the model, never searched
            if rec.is_meta {
                return out;
            }
            let mut texts = Vec::new();
            let mut results = 0;
            for p in pieces {
                match p.block {
                    Block::Text => texts.push(p.text),
                    Block::ToolResult(id) => {
                        results += 1;
                        if kinds.contains(Kind::ToolOutput) {
                            out.push(Item { kind: Kind::ToolOutput, sub: results, tool: Tool::Call(id), text: p.text });
                        }
                    }
                    _ => {}
                }
            }
            if let Some(kind) = user_kind.filter(|k| kinds.contains(*k))
                && !texts.is_empty()
            {
                out.insert(0, Item { kind, sub: 0, tool: Tool::None, text: join(texts) });
            }
        }
        Some("assistant") => {
            let mut texts = Vec::new();
            let mut thoughts = Vec::new();
            let mut calls = 0;
            for p in pieces {
                match p.block {
                    Block::Text => texts.push(p.text),
                    Block::Thinking if !p.text.trim().is_empty() => thoughts.push(p.text),
                    Block::ToolUse(name) => {
                        calls += 1;
                        if kinds.contains(Kind::ToolInput) && kinds.tool_matches(&name) {
                            let text = tool_input_text(p.tool_input.as_ref().expect("tool calls carry their input"));
                            out.push(Item { kind: Kind::ToolInput, sub: calls, tool: Tool::Name(name), text: Cow::Owned(text) });
                        }
                    }
                    _ => {}
                }
            }
            if kinds.contains(Kind::Reply) && !texts.is_empty() {
                out.insert(0, Item { kind: Kind::Reply, sub: 0, tool: Tool::None, text: join(texts) });
            }
            if kinds.contains(Kind::Thinking) && !thoughts.is_empty() {
                out.insert(0, Item { kind: Kind::Thinking, sub: 0, tool: Tool::None, text: join(thoughts) });
            }
        }
        _ => {}
    }
    out
}

fn join(mut parts: Vec<Cow<'_, str>>) -> Cow<'_, str> {
    if parts.len() == 1 { parts.pop().expect("one part") } else { Cow::Owned(parts.join("\n")) }
}

/// The snippet of a picked item: its line is parsed again for the text.
fn snippet(bytes: &[u8], f: Found, ctx: &Ctx) -> Hit {
    let start = f.item.offset as usize;
    let end = memchr(b'\n', &bytes[start..]).map_or(bytes.len(), |i| start + i);
    let line = bytes[start..end].strip_suffix(b"\r").unwrap_or(&bytes[start..end]);
    let user_kind = matches!(f.item.kind, Kind::Prompt | Kind::Other).then_some(f.item.kind);
    let text = serde_json::from_slice::<RawRecord>(line)
        .ok()
        .and_then(|rec| items(&rec, user_kind, ctx).into_iter().find(|i| i.kind == f.item.kind && i.sub == f.item.sub).map(|i| i.text.into_owned()))
        .unwrap_or_default();
    Hit {
        kind: f.item.kind,
        tool: match f.tool {
            Tool::Name(name) => Some(name),
            Tool::None | Tool::Call(_) => None,
        },
        turn: (f.turn > 0).then_some(f.turn),
        ts: f.ts,
        offset: f.item.offset,
        terms: indexes(f.item.mask),
        text: ctx.query.matcher().snippet(&text, SNIPPET_BEFORE, SNIPPET_AFTER),
    }
}

/// Name the tool results among `found` after their calls: one search of the file for the calls'
/// ids, and a parse of each line that has one.
fn name_tools(bytes: &[u8], found: Vec<&mut Found>) {
    let ids: HashSet<&str> = found.iter().filter_map(|f| match &f.tool {
        Tool::Call(Some(id)) => Some(id.as_str()),
        _ => None,
    }).collect();
    if ids.is_empty() {
        return;
    }
    let ids: Vec<String> = ids.into_iter().map(String::from).collect();
    let patterns: Vec<String> = ids.iter().map(|id| format!("\"id\":\"{id}\"")).collect();
    let ac = AhoCorasick::new(&patterns).expect("literal patterns build");
    let mut names: HashMap<String, String> = HashMap::new();
    let mut parsed: HashSet<usize> = HashSet::new();
    for m in ac.find_iter(bytes) {
        let start = memrchr(b'\n', &bytes[..m.start()]).map_or(0, |i| i + 1);
        if !parsed.insert(start) {
            continue;
        }
        let end = memchr(b'\n', &bytes[start..]).map_or(bytes.len(), |i| start + i);
        let Ok(rec) = serde_json::from_slice::<RawRecord>(&bytes[start..end]) else { continue };
        let Some(Content::Blocks(blocks)) = rec.message.as_ref().and_then(|m| m.content).map(parse_content) else { continue };
        for b in blocks {
            if b.kind.as_deref() == Some("tool_use")
                && let (Some(id), Some(name)) = (b.id.as_deref(), b.name.as_deref())
            {
                names.entry(id.to_string()).or_insert_with(|| name.to_string());
            }
        }
    }
    for f in found {
        if let Tool::Call(Some(id)) = &f.tool {
            f.tool = match names.get(id) {
                Some(name) => Tool::Name(name.clone()),
                None => Tool::Call(None),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    /// Threads holding leases never hold more than the budget together; a request bigger than the
    /// budget waits for all of it.
    #[test]
    fn leases_never_hold_more_than_the_budget() {
        let budget = Budget::new(250);
        let (held, peak) = (AtomicU64::new(0), AtomicU64::new(0));
        std::thread::scope(|scope| {
            for i in 0..12 {
                let (budget, held, peak) = (&budget, &held, &peak);
                scope.spawn(move || {
                    let want = if i % 4 == 0 { 1_000 } else { 100 };
                    let lease = budget.take(want);
                    let now = held.fetch_add(lease.bytes, Ordering::SeqCst) + lease.bytes;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    held.fetch_sub(lease.bytes, Ordering::SeqCst);
                });
            }
        });
        assert!(peak.load(Ordering::SeqCst) <= 250, "peak {}", peak.load(Ordering::SeqCst));
        assert_eq!(*budget.left.lock().unwrap(), 250, "every lease was given back");
        assert_eq!(budget.take(1_000).bytes, 250, "a big file takes the whole budget");
    }
}
