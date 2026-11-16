//! The reading options of a transcript export (`asm transcript --outline / --turns / --grep /
//! --thinking / --tools`): the turns asked for, each with every item `asm search` searches in it,
//! so that a hit the search reports can be read in its turn.
//!
//! Turns are numbered by the shared rule (`RecordFilter` + `opens_turn`), like the index and the
//! detail panel. The items are the search's: a user record's text as recorded (a prompt keeps the
//! tags Claude Code wraps around it), each assistant record's reply and thinking, each tool call's
//! input as the `tool-input` kind writes it and its result as `tool-output` reads it. Like the
//! index and the scan, every record counts, also the ones the full export and the detail panel
//! leave out (a later copy of a record, a sidechain record in a main transcript); none were found
//! in real transcripts (2026-11-16).

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{Message, Record, RecordFilter, TranscriptOptions, opens_turn, sanitize_text, xml_attribute};
use crate::claude::clean::clean_prompt;
use crate::claude::parse::{Content, user_text};
use crate::format::{fmt_datetime, parse_ts};
use crate::model::TranscriptExport;
use crate::search::Matcher;
use crate::search::query::Query;
use crate::search::regex::{tool_input_map, tool_result_text};
use crate::search::sessions::tool_input_text;

/// An `<input>`, `<output>` or `<other>` body longer than this many characters is cut.
pub const CUT_CHARS: usize = 2_000;
/// With a hit inside, the cut body starts with this many characters...
const HEAD_CHARS: usize = 500;
/// ...and keeps this many before each hit and from its start...
const BEFORE_HIT: usize = 300;
const FROM_HIT: usize = 700;
/// ...up to this many in all.
const KEEP_CHARS: usize = 4_000;
/// Characters of a prompt's first line in the outline.
const OUTLINE_CHARS: usize = 120;
const MISSING: &str = "<agent status=\"missing\">\n[No final response recorded]\n</agent>\n";

pub(super) fn read(path: &Path, opts: &TranscriptOptions) -> Result<TranscriptExport> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut reader = Reader::new(opts);
    let mut filter = RecordFilter::for_file(path);
    let mut skipped_lines = 0;
    let mut lines = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        if lines.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice::<Record>(&line) {
            Ok(record) => reader.record(&record, &mut filter),
            Err(_) => skipped_lines += 1,
        }
    }
    reader.finish();
    let total = reader.total;
    if let Some(range) = opts.turns {
        let past = if range.first > total { Some(range.first) } else { range.last.filter(|last| *last > total) };
        if let Some(turn) = past {
            bail!("turn {turn} is beyond the last turn ({total})");
        }
    }
    let id = path.file_stem().unwrap_or_default().to_string_lossy();
    let text = if opts.outline { reader.outline(&id) } else { reader.xml(&id) };
    Ok(TranscriptExport { text, skipped_lines })
}

/// A tool call's result.
struct Output {
    text: String,
    error: bool,
}

/// What a turn shows after its prompt, in file order.
enum Part {
    Thinking(String),
    Agent(String),
    Other(String),
    /// A call (`input`) and its result (`output`) when it came; a result whose call is not in the
    /// turn has no input.
    Tool { name: String, input: Option<String>, output: Option<Output> },
}

struct Turn {
    number: u32,
    ts: Option<i64>,
    /// As recorded: the text `asm search` searches.
    prompt: String,
    media: Vec<&'static str>,
    parts: Vec<Part>,
    /// The part of each call, by call id, for its result.
    calls: HashMap<String, usize>,
    tool_counts: BTreeMap<String, usize>,
    /// The agent wrote anything at all in the turn.
    answered: bool,
    /// The full export's rule: the last thing the agent wrote is text without a tool call.
    final_reply: bool,
}

impl Turn {
    /// Any TERM in the prompt or a part shown (thinking and tools are parts only when asked for).
    fn has_hit(&self, query: &Query) -> bool {
        let hit = |text: &str| query.mask(text) != 0;
        hit(&self.prompt)
            || self.parts.iter().any(|part| match part {
                Part::Thinking(text) | Part::Agent(text) | Part::Other(text) => hit(text),
                Part::Tool { input, output, .. } => input.as_deref().is_some_and(hit) || output.as_ref().is_some_and(|o| hit(&o.text)),
            })
    }

    /// The prompt as recorded, with a placeholder per image or document.
    fn prompt_body(&self) -> String {
        let mut text = self.prompt.trim().to_string();
        for placeholder in &self.media {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(placeholder);
        }
        text
    }

    fn xml(&self, opts: &TranscriptOptions) -> String {
        let matcher = opts.grep.as_ref().map(Query::matcher);
        let ts = self.ts.map(|ms| format!(" ts=\"{}\"", xml_attribute(&fmt_datetime(Some(ms))))).unwrap_or_default();
        let mut out = format!("<user turn=\"{}\"{ts}>\n{}\n</user>\n\n", self.number, sanitize_text(&self.prompt_body()));
        let mut parts = String::new();
        if !opts.tools && !self.tool_counts.is_empty() {
            let calls = self.tool_counts.iter().map(|(name, count)| {
                let times = if *count == 1 { "time" } else { "times" };
                format!("{name} {count} {times}")
            });
            parts.push_str(&format!("<turn-summary>\nCalled {}.\n</turn-summary>\n", sanitize_text(&calls.collect::<Vec<_>>().join(", "))));
        }
        for part in &self.parts {
            match part {
                Part::Thinking(text) => element(&mut parts, "thinking", "thinking", text.trim()),
                Part::Agent(text) => element(&mut parts, "agent", "agent", text.trim()),
                Part::Other(text) => element(&mut parts, "other", "other", &cut(text.trim(), matcher)),
                Part::Tool { name, input, output } => {
                    parts.push_str(&format!("<tool name=\"{}\">\n", xml_attribute(name)));
                    if let Some(input) = input {
                        element(&mut parts, "input", "input", &cut(input.trim(), matcher));
                    }
                    if let Some(output) = output {
                        let open = if output.error { "output status=\"error\"" } else { "output" };
                        element(&mut parts, open, "output", &cut(output.text.trim(), matcher));
                    }
                    parts.push_str("</tool>\n");
                }
            }
        }
        if self.answered && !self.final_reply {
            parts.push_str(MISSING);
        }
        if !parts.is_empty() {
            out.push_str(&parts);
            out.push('\n');
        }
        out
    }

    /// `   5  2026-11-13 08:01  the first line of the prompt`, cleaned of injected tags.
    fn outline(&self) -> String {
        let cleaned = clean_prompt(&self.prompt);
        let first = cleaned.lines().map(str::trim).find(|l| !l.is_empty()).or_else(|| self.media.first().copied()).unwrap_or("");
        let mut line: String = first.chars().take(OUTLINE_CHARS).collect();
        if first.chars().nth(OUTLINE_CHARS).is_some() {
            line.push('…');
        }
        format!("{:>4}  {:<16}  {}\n", self.number, fmt_datetime(self.ts), sanitize_text(&line))
    }
}

fn element(out: &mut String, open: &str, close: &str, body: &str) {
    out.push_str(&format!("<{open}>\n{}\n</{close}>\n", sanitize_text(body)));
}

/// Turns kept so far, and the ones that may still be kept as context of a later hit.
#[derive(Default)]
struct Kept {
    text: String,
    count: u32,
    /// The last turns not kept (up to the context), rendered when in range.
    pending: VecDeque<Option<String>>,
    /// Turns still to keep after the last turn with a hit.
    after: u32,
}

impl Kept {
    fn push(&mut self, text: String) {
        self.text.push_str(&text);
        self.count += 1;
    }
}

struct Reader<'o> {
    opts: &'o TranscriptOptions,
    total: u32,
    turn: Option<Turn>,
    /// Tool names by call id, for a result that comes in a later turn than its call.
    names: HashMap<String, String>,
    first_ts: Option<i64>,
    last_ts: Option<i64>,
    kept: Kept,
}

impl<'o> Reader<'o> {
    fn new(opts: &'o TranscriptOptions) -> Self {
        Reader { opts, total: 0, turn: None, names: HashMap::new(), first_ts: None, last_ts: None, kept: Kept::default() }
    }

    fn record(&mut self, r: &Record, filter: &mut RecordFilter) {
        if let Some(ts) = r.timestamp.as_deref().and_then(parse_ts) {
            self.last_ts = Some(self.last_ts.map_or(ts, |last| last.max(ts)));
        }
        let content = r.content();
        let user_input = r.is_user_input(content.as_ref());
        // every record takes its uuid, so the turns are numbered like the index's
        let kept = filter.keep(r.uuid.as_deref(), r.is_sidechain, user_input);
        let (Some(message), Some(content)) = (&r.message, content) else { return };
        match r.kind.as_deref() {
            Some("user") => self.user(r, &content, user_input, kept),
            Some("assistant") => self.assistant(r, message, &content),
            _ => {}
        }
    }

    fn user(&mut self, r: &Record, content: &Content, user_input: bool, kept: bool) {
        // injected for the model, never searched
        if r.is_meta {
            return;
        }
        let ts = r.timestamp.as_deref().and_then(parse_ts);
        if user_input {
            let text = user_text(content).unwrap_or_default();
            if kept && opens_turn(&r.user_flags(), content) {
                self.start(ts, text, media(content));
                return;
            }
            if let Some(turn) = &mut self.turn
                && !text.trim().is_empty()
            {
                turn.parts.push(Part::Other(text));
            }
        }
        if !self.opts.tools {
            return;
        }
        let (Some(turn), Content::Blocks(blocks)) = (&mut self.turn, content) else { return };
        for block in blocks.iter().filter(|b| b.kind.as_deref() == Some("tool_result")) {
            let output = Output { text: tool_result_text(block.content), error: block.is_error.unwrap_or(false) };
            let id = block.tool_use_id.as_deref();
            match id.and_then(|id| turn.calls.get(id)).map(|i| &mut turn.parts[*i]) {
                Some(Part::Tool { output: slot @ None, .. }) => *slot = Some(output),
                _ => {
                    let name = id.and_then(|id| self.names.get(id)).cloned().unwrap_or_else(|| "?".into());
                    turn.parts.push(Part::Tool { name, input: None, output: Some(output) });
                }
            }
        }
    }

    fn assistant(&mut self, r: &Record, message: &Message, content: &Content) {
        let opts = self.opts;
        let Some(turn) = &mut self.turn else { return };
        turn.answered = true;
        let blocks = match content {
            Content::Blocks(blocks) => blocks.as_slice(),
            _ => &[],
        };
        if opts.thinking {
            let thoughts: Vec<&str> = blocks
                .iter()
                .filter(|b| b.kind.as_deref() == Some("thinking"))
                .filter_map(|b| b.thinking.as_deref())
                .filter(|t| !t.trim().is_empty())
                .collect();
            if !thoughts.is_empty() {
                turn.parts.push(Part::Thinking(thoughts.join("\n")));
            }
        }
        let reply = match content {
            Content::Text(text) => Some(text.to_string()),
            Content::Blocks(blocks) => {
                let texts: Vec<&str> = blocks.iter().filter(|b| b.kind.as_deref() == Some("text")).filter_map(|b| b.text.as_deref()).collect();
                (!texts.is_empty()).then(|| texts.join("\n"))
            }
            Content::Other => None,
        };
        if let Some(text) = reply.filter(|t| !t.trim().is_empty()) {
            turn.parts.push(Part::Agent(text));
        }
        let mut has_tools = message.stop_reason.as_deref() == Some("tool_use");
        for block in blocks.iter().filter(|b| b.kind.as_deref() == Some("tool_use")) {
            has_tools = true;
            let name = block.name.as_deref().unwrap_or("?").to_string();
            *turn.tool_counts.entry(name.clone()).or_default() += 1;
            if opts.tools {
                let input = tool_input_text(&tool_input_map(block.input));
                if let Some(id) = block.id.as_deref() {
                    turn.calls.insert(id.to_string(), turn.parts.len());
                    self.names.insert(id.to_string(), name.clone());
                }
                turn.parts.push(Part::Tool { name, input: Some(input), output: None });
            }
        }
        if r.is_aborted || has_tools {
            turn.final_reply = false;
        } else if user_text(content).is_some_and(|t| !t.trim().is_empty()) {
            turn.final_reply = true;
        }
    }

    fn start(&mut self, ts: Option<i64>, prompt: String, media: Vec<&'static str>) {
        self.finish();
        self.total += 1;
        if self.total == 1 {
            self.first_ts = ts;
        }
        self.turn = Some(Turn {
            number: self.total,
            ts,
            prompt,
            media,
            parts: Vec::new(),
            calls: HashMap::new(),
            tool_counts: BTreeMap::new(),
            answered: false,
            final_reply: false,
        });
    }

    /// Keep the current turn, hold it as possible context, or drop it.
    fn finish(&mut self) {
        let Some(turn) = self.turn.take() else { return };
        let opts = self.opts;
        let in_range = opts.turns.is_none_or(|r| r.contains(turn.number));
        let context = if opts.grep.is_some() { opts.context } else { 0 };
        let render = |turn: &Turn| if opts.outline { turn.outline() } else { turn.xml(opts) };
        if opts.grep.as_ref().is_none_or(|q| turn.has_hit(q)) {
            while let Some(before) = self.kept.pending.pop_front() {
                if let Some(text) = before {
                    self.kept.push(text);
                }
            }
            self.kept.after = context;
            if in_range {
                self.kept.push(render(&turn));
            }
        } else if self.kept.after > 0 {
            self.kept.after -= 1;
            if in_range {
                self.kept.push(render(&turn));
            }
        } else if context > 0 {
            self.kept.pending.push_back(in_range.then(|| render(&turn)));
            if self.kept.pending.len() > context as usize {
                self.kept.pending.pop_front();
            }
        }
    }

    fn xml(self, id: &str) -> String {
        let matched = if self.opts.grep.is_some() { format!(" matched=\"{}\"", self.kept.count) } else { String::new() };
        format!("<transcript session_id=\"{}\" turns=\"{}\"{matched}>\n\n{}</transcript>\n", xml_attribute(id), self.total, self.kept.text)
    }

    /// `ID · 21 turns, 2 matched · 2026-11-13 07:40 to 2026-11-14 17:09`, then a line per turn.
    fn outline(self, id: &str) -> String {
        let mut head = format!("{id} · {} turn{}", self.total, if self.total == 1 { "" } else { "s" });
        if self.opts.grep.is_some() {
            head.push_str(&format!(", {} matched", self.kept.count));
        }
        if let Some(first) = self.first_ts {
            head.push_str(&format!(" · {} to {}", fmt_datetime(Some(first)), fmt_datetime(self.last_ts.or(Some(first)))));
        }
        format!("{}\n{}", sanitize_text(&head), self.kept.text)
    }
}

fn media(content: &Content) -> Vec<&'static str> {
    let Content::Blocks(blocks) = content else { return Vec::new() };
    blocks
        .iter()
        .filter_map(|b| match b.kind.as_deref() {
            Some("image") => Some("[Image omitted]"),
            Some("document") => Some("[Document omitted]"),
            _ => None,
        })
        .collect()
}

/// A body as shown: whole up to `CUT_CHARS` characters. Longer, without a hit inside: the first
/// `CUT_CHARS`. With hits: the first `HEAD_CHARS` and a window around each hit, windows that touch
/// merged, `KEEP_CHARS` in all; a hit past that gets no window. Each gap is a line
/// `…[cut N chars]`, the last one also counting the hits it hides.
fn cut<'t>(text: &'t str, matcher: Option<&Matcher>) -> Cow<'t, str> {
    let n = text.chars().count();
    if n <= CUT_CHARS {
        return Cow::Borrowed(text);
    }
    let starts: Vec<usize> = matcher.map(|m| m.ranges(text).into_iter().map(|r| r.start).collect()).unwrap_or_default();
    let hits = char_offsets(text, &starts);
    let mut windows = vec![(0, if hits.is_empty() { CUT_CHARS } else { HEAD_CHARS })];
    let mut kept = windows[0].1;
    let mut hidden = 0;
    let mut full = false;
    for hit in hits {
        let last = windows.last_mut().expect("the head");
        if full {
            hidden += usize::from(hit >= last.1);
            continue;
        }
        let (start, end) = (hit.saturating_sub(BEFORE_HIT), (hit + FROM_HIT).min(n));
        let touches = start <= last.1;
        let added = if touches { end.saturating_sub(last.1) } else { end - start };
        if kept + added > KEEP_CHARS {
            full = true;
            hidden += usize::from(hit >= last.1);
            continue;
        }
        kept += added;
        if touches {
            last.1 = last.1.max(end);
        } else {
            windows.push((start, end));
        }
    }
    let bounds: Vec<usize> = windows.iter().flat_map(|(s, e)| [*s, *e]).collect();
    let bytes = byte_offsets(text, &bounds);
    let mut out = String::new();
    let mut pos = 0;
    for (i, (start, end)) in windows.iter().enumerate() {
        if *start > pos {
            gap(&mut out, start - pos, 0);
            out.push('\n');
        }
        out.push_str(&text[bytes[2 * i]..bytes[2 * i + 1]]);
        pos = *end;
    }
    if pos < n {
        gap(&mut out, n - pos, hidden);
    }
    Cow::Owned(out)
}

fn gap(out: &mut String, chars: usize, hidden: usize) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("…[cut {chars} chars"));
    if hidden > 0 {
        out.push_str(&format!(", {hidden} more hit{}", if hidden == 1 { "" } else { "s" }));
    }
    out.push(']');
}

/// The character offsets of ascending byte offsets (each at a character boundary).
fn char_offsets(text: &str, bytes: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut wanted = bytes.iter().peekable();
    for (chars, (at, _)) in text.char_indices().chain(std::iter::once((text.len(), ' '))).enumerate() {
        while wanted.next_if(|b| **b <= at).is_some() {
            out.push(chars);
        }
        if wanted.peek().is_none() {
            break;
        }
    }
    out
}

/// The byte offsets of ascending character offsets (at most the text's length in characters).
fn byte_offsets(text: &str, chars: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(chars.len());
    let mut wanted = chars.iter().peekable();
    for (i, (at, _)) in text.char_indices().chain(std::iter::once((text.len(), ' '))).enumerate() {
        while wanted.next_if(|c| **c <= i).is_some() {
            out.push(at);
        }
        if wanted.peek().is_none() {
            break;
        }
    }
    out
}
