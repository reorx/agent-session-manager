//! Turn cards for the Conversation tab: a core `Conversation` (read straight from the transcript)
//! turned into pre-formatted cards, and the row table the detail list renders. A row is one
//! section of a card (`Row`), so an open turn with 200 steps is 200 small list items and a hit
//! can be scrolled to the very step that holds it.
//!
//! Every string a card shows is made here, once, on the reader thread (`store.rs`); rendering
//! only picks them up, like `render_td` does for the table.

use std::collections::HashSet;
use std::ops::Range;

use agent_session_core::claude::paths::home_dir;
use agent_session_core::model::{Compaction, ConvItem, Conversation, HitPlace, Marked, Part, Step, StepKind, ToolCall, ToolResult, Turn, TurnEnd};
use agent_session_core::search::regex::{ToolLine, ToolLineKind};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use gpui_kit::SharedString;

/// Source lines a folded prompt shows.
pub const PROMPT_LINES: usize = 4;
/// Source lines a folded reply or process text shows.
pub const REPLY_LINES: usize = 6;
/// Source lines a folded tool output shows.
pub const OUTPUT_LINES: usize = 12;
/// Source lines of an API error at the bottom of a card.
const ERROR_LINES: usize = 2;
/// Characters a folded text shows at most, for long stretches without a line break.
pub const FOLD_CHARS: usize = 900;
/// Tool argument paths longer than this lose their middle.
const PATH_CHARS: usize = 60;
/// Characters of a thinking or process text kept in its one-line opening.
const OPENING_CHARS: usize = 120;
/// Tool outputs at least this big show their size next to the line count.
const SIZE_SHOWN_BYTES: usize = 512;

/// A session's conversation, ready to render.
#[derive(Debug, Default)]
pub struct ConversationView {
    /// `9 turns · 214 tool calls`, or `21 turns · 2 auto · 208 tool calls`.
    pub summary: SharedString,
    /// One per core `ConvItem`, same index, so a `HitPlace::item` is a card index.
    pub cards: Vec<Card>,
    /// Every place with a search hit, in file order.
    pub hits: Vec<HitPlace>,
    /// `Matcher::pattern()` of the search the transcript was read with.
    pub pattern: Option<String>,
    /// Transcript size at read time.
    pub bytes: u64,
}

#[derive(Debug)]
pub enum Card {
    Turn(TurnCard),
    Compact(CompactCard),
}

/// The lamp in a card's title line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Ended with a reply.
    Ok,
    /// Interrupted by the user, or an API error.
    Failed,
    /// No reply (still running, or the process went away).
    Off,
}

#[derive(Debug)]
pub struct TurnCard {
    /// `TURN 3` or `AUTO`.
    pub label: SharedString,
    pub auto: bool,
    /// Where an AUTO card came from (`task-notification`, `session start`, …); empty for turns.
    pub origin: SharedString,
    /// `10:24`, or `10-17 21:40` when the day changed since the card before (or it isn't today).
    pub time: SharedString,
    pub status: Status,
    /// `19 steps · 6m 30s`.
    pub meta: SharedString,
    /// `1 error`, red; empty without failed tool calls.
    pub errors: SharedString,
    /// `interrupted` / `API error` (red) or `no reply` (gray); empty when the turn ended normally.
    pub end_note: SharedString,
    pub prompt: Folded,
    /// `image · 1.2 MB`, one per pasted image or document.
    pub attachments: Vec<SharedString>,
    /// One per step, in order.
    pub ticks: Vec<Tick>,
    /// `Read 12 · Bash 11 · text 2 · thinking 3`; empty without steps.
    pub stats: SharedString,
    pub steps: Vec<StepView>,
    pub tail: Tail,
    /// Hit places in this card.
    pub hits: usize,
    /// `2 hits`; empty without hits.
    pub hits_fmt: SharedString,
}

#[derive(Debug)]
pub struct CompactCard {
    /// `context compacted · 22:05`.
    pub label: SharedString,
    /// `summary · 9.8k chars`; empty when the summary is missing.
    pub summary_label: SharedString,
    pub summary: Option<Folded>,
    pub hits: usize,
}

/// A text block that may start folded. Folding counts source lines: a long line that wraps on
/// screen is still one line.
#[derive(Debug, Default)]
pub struct Folded {
    pub text: SharedString,
    /// Byte ranges of search hits in `text`.
    pub hits: Vec<Range<usize>>,
    /// Where the folded text stops (a byte position in `text`); `None` shows it whole, either
    /// because it is short or because a hit lies past the fold.
    pub fold: Option<usize>,
    /// What the unfold link says: `Show all · 18 lines`, or `+ 40 more lines · 4.1 KB` under a
    /// tool output. Empty when `fold` is `None`.
    pub more: SharedString,
    /// `text` up to the fold (all of it without one), so rendering does not cut strings; the
    /// hits are all inside it when there is a fold.
    pub short: SharedString,
}

impl Folded {
    fn new(text: &str, hits: Vec<Range<usize>>, fold: Option<usize>, more: SharedString) -> Folded {
        // a fold right after a blank line would leave an empty line above the unfold link
        let short = fold.map(|at| {
            let end = hits.iter().map(|hit| hit.end).fold(text[..at].trim_end().len(), usize::max);
            SharedString::from(text[..end].to_string())
        });
        let text: SharedString = text.to_string().into();
        Folded { short: short.unwrap_or_else(|| text.clone()), text, hits, fold, more }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickKind {
    /// A bar, red when the call failed.
    Tool,
    /// A filled dot.
    Text,
    /// A hollow dot.
    Thinking,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    pub kind: TickKind,
    pub error: bool,
    /// Ringed in yellow; the current hit's ring (orange) is the detail state's business.
    pub hit: bool,
}

/// One step, both as its collapsed row and as what it shows open. `label`, `summary` and `meta`
/// also make up the tick hover line: `14 / 29  Bash  cargo test …  error · 46 lines`.
#[derive(Debug)]
pub struct StepView {
    pub kind: TickKind,
    pub time: SharedString,
    /// Tool name (`?` for a result whose call is missing), `thinking` or `text`.
    pub label: SharedString,
    /// Tool argument (paths shortened) or the opening of the text.
    pub summary: SharedString,
    /// Tool result (`212 lines`, `ok`, `error · 46 lines`) or a thinking's length (`1.2k chars`).
    pub meta: SharedString,
    /// A failed tool call: red tick, red result, red output.
    pub error: bool,
    /// Search hits (occurrences) in the step; `hits_fmt` is the badge at the end of its row.
    pub hits: usize,
    pub hits_fmt: SharedString,
    pub body: StepBody,
}

#[derive(Debug)]
pub enum StepBody {
    Tool { input: Vec<InputLine>, output: Option<Folded> },
    /// Shown whole once the step is open.
    Thinking(Folded),
    /// Process text, shown like a reply (folded at `REPLY_LINES`), never collapsed to a row.
    Text(Folded),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    /// `# description`, muted.
    Description,
    /// `$ command`.
    Command,
    /// `key: value`.
    Field,
}

/// One line of a tool's input box, prefix included.
#[derive(Debug)]
pub struct InputLine {
    pub kind: InputKind,
    pub text: SharedString,
    pub hits: Vec<Range<usize>>,
}

/// The bottom of a turn card, above the Show steps toggle.
#[derive(Debug)]
pub enum Tail {
    Reply { time: SharedString, text: Folded },
    /// `[Request interrupted by user]`, gray.
    Interrupted { time: SharedString },
    /// Red, folded at two lines.
    ApiError { time: SharedString, text: Folded },
    /// No reply: only the toggle (if there are steps) and the card's bottom edge.
    Empty,
}

/// One item of the detail list: a section of a card. The card's look comes from its rows put
/// together: `Head` draws the top edge, `Tail` the bottom one, steps only the sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Row {
    /// Title line, prompt, attachments, ticks and stats.
    Head(usize),
    /// `(card, step)`; only while the turn is open.
    Step(usize, usize),
    /// Reply / interruption / API error, and the Show steps toggle.
    Tail(usize),
    /// A compaction divider.
    Compact(usize),
}

impl Row {
    /// The card this row is a section of.
    pub fn card(self) -> usize {
        match self {
            Row::Head(card) | Row::Step(card, _) | Row::Tail(card) | Row::Compact(card) => card,
        }
    }
}

pub fn build(conv: &Conversation, project_path: &str, today: NaiveDate) -> ConversationView {
    let hits = conv.hit_places();
    let mut places = vec![0; conv.items.len()];
    for place in &hits {
        places[place.item] += 1;
    }
    let home = home_dir().to_string_lossy().into_owned();
    let paths = Paths { project: project_path.trim_end_matches('/'), home: home.trim_end_matches('/') };
    // a card says its date when the day differs from the card before; the first one compares with today
    let mut day = today;
    let cards = conv
        .items
        .iter()
        .zip(places)
        .map(|(item, hits)| {
            let ts = match item {
                ConvItem::Turn(t) => t.ts,
                ConvItem::Compact(c) => c.ts,
            };
            let time = time_label(ts, day);
            day = date_of(ts).unwrap_or(day);
            match item {
                ConvItem::Turn(t) => Card::Turn(turn_card(t, time, day, hits, &paths)),
                ConvItem::Compact(c) => Card::Compact(compact_card(c, time, hits)),
            }
        })
        .collect();
    ConversationView { summary: summary(conv).into(), cards, hits, pattern: None, bytes: conv.bytes }
}

/// The row table: every card's rows in order, open turns with their steps between head and tail.
pub fn rows(view: &ConversationView, open: &HashSet<usize>) -> Vec<Row> {
    let mut rows = Vec::with_capacity(view.cards.len() * 2);
    for (ix, card) in view.cards.iter().enumerate() {
        push_rows(&mut rows, ix, card, open.contains(&ix));
    }
    rows
}

/// One card's rows; what `ListState::splice` puts in when the card opens or closes.
pub fn card_rows(ix: usize, card: &Card, open: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    push_rows(&mut rows, ix, card, open);
    rows
}

fn push_rows(rows: &mut Vec<Row>, ix: usize, card: &Card, open: bool) {
    match card {
        Card::Compact(_) => rows.push(Row::Compact(ix)),
        Card::Turn(turn) => {
            rows.push(Row::Head(ix));
            if open {
                rows.extend((0..turn.steps.len()).map(|step| Row::Step(ix, step)));
            }
            rows.push(Row::Tail(ix));
        }
    }
}

/// The row that shows `place`: a step of a closed turn is under the turn's head.
pub fn row_of(rows: &[Row], place: &HitPlace) -> Option<usize> {
    let item = place.item;
    let find = |row: Row| rows.iter().position(|r| *r == row);
    match place.part {
        Part::Prompt => find(Row::Head(item)),
        Part::Step(step) => find(Row::Step(item, step)).or_else(|| find(Row::Head(item))),
        Part::Reply => find(Row::Tail(item)),
        Part::Summary => find(Row::Compact(item)),
    }
}

/// Where `text` folds to show at most `max_lines` source lines and `max_chars` characters;
/// `None` when it fits, or when a hit lies past the fold (the text then opens by itself).
pub fn fold(text: &str, hits: &[Range<usize>], max_lines: usize, max_chars: usize) -> Option<usize> {
    let more_after = |at: &usize| !text[*at..].trim().is_empty();
    let by_lines = text.match_indices('\n').nth(max_lines.saturating_sub(1)).map(|(at, _)| at).filter(more_after);
    let by_chars = text.char_indices().nth(max_chars).map(|(at, _)| at).filter(more_after);
    let at = match (by_lines, by_chars) {
        (Some(a), Some(b)) => a.min(b),
        (a, b) => a.or(b)?,
    };
    (!hits.iter().any(|hit| hit.end > at)).then_some(at)
}

/// Project and home directories, to shorten the paths tool arguments show.
struct Paths<'a> {
    project: &'a str,
    home: &'a str,
}

impl Paths<'_> {
    /// Relative inside the project, `~/…` inside home, the middle cut out when still too long.
    fn short(&self, path: &str) -> String {
        let under = |dir: &str| (!dir.is_empty() && dir != "/").then(|| path.strip_prefix(dir)).flatten().filter(|rest| rest.starts_with('/'));
        let shown = if path == self.project {
            ".".to_string()
        } else if let Some(rest) = under(self.project) {
            rest[1..].to_string()
        } else if let Some(rest) = under(self.home) {
            format!("~{rest}")
        } else {
            path.to_string()
        };
        elide_middle(&shown, PATH_CHARS)
    }
}

/// `~/.cargo/registry/…/gpui-component-0.6.1/src/date_picker.rs`: the file name and the parents
/// that fit in two thirds of `max`, then as many leading directories as still fit.
fn elide_middle(path: &str, max: usize) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if path.chars().count() <= max || parts.len() < 3 {
        return path.to_string();
    }
    let width = |parts: &[&str]| parts.iter().map(|p| p.chars().count() + 1).sum::<usize>();
    let mut tail = parts.len() - 1;
    while tail > 1 && width(&parts[tail - 1..]) <= max * 2 / 3 {
        tail -= 1;
    }
    let mut head = 0;
    while head + 1 < tail && width(&parts[..head + 1]) + width(&parts[tail..]) + 2 <= max {
        head += 1;
    }
    let tail = parts[tail..].join("/");
    if head == 0 { format!("…/{tail}") } else { format!("{}/…/{tail}", parts[..head].join("/")) }
}

fn summary(conv: &Conversation) -> String {
    let (mut turns, mut autos, mut tools) = (0, 0, 0);
    for item in &conv.items {
        let ConvItem::Turn(t) = item else { continue };
        if t.number.is_some() { turns += 1 } else { autos += 1 }
        tools += t.steps.iter().filter(|s| matches!(s.kind, StepKind::Tool(_))).count();
    }
    let mut parts = vec![count(turns, "turn", "turns")];
    if autos > 0 {
        parts.push(format!("{autos} auto"));
    }
    parts.push(count(tools, "tool call", "tool calls"));
    parts.join(" · ")
}

fn turn_card(t: &Turn, time: SharedString, day: NaiveDate, hits: usize, paths: &Paths) -> TurnCard {
    let steps: Vec<StepView> = t.steps.iter().map(|step| step_view(step, day, paths)).collect();
    let ticks = steps.iter().map(|s| Tick { kind: s.kind, error: s.error, hit: s.hits > 0 }).collect();
    let mut meta = Vec::new();
    if !t.steps.is_empty() {
        meta.push(count(t.steps.len(), "step", "steps"));
    }
    meta.extend(t.duration_ms.or_else(|| Some(t.last_ts? - t.ts?)).and_then(duration_label));
    let errors = steps.iter().filter(|s| s.error).count();
    let (status, end_note) = match t.end {
        TurnEnd::Completed => (Status::Ok, ""),
        TurnEnd::Interrupted { .. } => (Status::Failed, "interrupted"),
        TurnEnd::ApiError { .. } => (Status::Failed, "API error"),
        TurnEnd::NoReply => (Status::Off, "no reply"),
    };
    let tail = match (&t.end, &t.reply) {
        (TurnEnd::Interrupted { ts }, _) => Tail::Interrupted { time: time_label(*ts, day) },
        (TurnEnd::ApiError { ts, text }, _) => Tail::ApiError { time: time_label(*ts, day), text: plain_folded(text, ERROR_LINES) },
        (_, Some(reply)) => Tail::Reply { time: time_label(reply.ts, day), text: folded(&reply.text, REPLY_LINES) },
        (_, None) => Tail::Empty,
    };
    TurnCard {
        label: t.number.map_or_else(|| "AUTO".to_string(), |n| format!("TURN {n}")).into(),
        auto: t.number.is_none(),
        origin: t.number.is_none().then(|| t.origin.clone()).flatten().unwrap_or_default().into(),
        time,
        status,
        meta: meta.join(" · ").into(),
        errors: if errors == 0 { SharedString::default() } else { count(errors, "error", "errors").into() },
        end_note: end_note.into(),
        prompt: folded(&t.prompt, PROMPT_LINES),
        attachments: t.attachments.iter().map(|a| format!("{} · {}", a.kind, size_label(a.bytes)).into()).collect(),
        ticks,
        stats: stats(&t.steps).into(),
        steps,
        tail,
        hits,
        hits_fmt: if hits == 0 { SharedString::default() } else { count(hits, "hit", "hits").into() },
    }
}

fn compact_card(c: &Compaction, time: SharedString, hits: usize) -> CompactCard {
    CompactCard {
        label: if time.is_empty() { "context compacted".into() } else { format!("context compacted · {time}").into() },
        summary_label: c.summary.as_ref().map_or_else(SharedString::default, |m| format!("summary · {}", chars_label(m.chars)).into()),
        summary: c.summary.as_ref().map(whole),
        hits,
    }
}

/// `Read 12 · Bash 11 · text 2 · thinking 3`: tools by count (ties by name), then the texts.
fn stats(steps: &[Step]) -> String {
    let mut tools: Vec<(&str, usize)> = Vec::new();
    let (mut texts, mut thinking) = (0, 0);
    for step in steps {
        match &step.kind {
            StepKind::Tool(call) => match tools.iter_mut().find(|(name, _)| *name == call.name) {
                Some((_, n)) => *n += 1,
                None => tools.push((&call.name, 1)),
            },
            StepKind::Text(_) => texts += 1,
            StepKind::Thinking(_) => thinking += 1,
        }
    }
    tools.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let mut parts: Vec<String> = tools.iter().map(|(name, n)| format!("{name} {n}")).collect();
    parts.extend((texts > 0).then(|| format!("text {texts}")));
    parts.extend((thinking > 0).then(|| format!("thinking {thinking}")));
    parts.join(" · ")
}

fn step_view(step: &Step, day: NaiveDate, paths: &Paths) -> StepView {
    let time = time_label(step.ts, day);
    match &step.kind {
        StepKind::Tool(call) => tool_view(call, time, paths),
        StepKind::Thinking(m) => StepView {
            kind: TickKind::Thinking,
            time,
            label: "thinking".into(),
            summary: opening(&m.text).into(),
            meta: chars_label(m.chars).into(),
            error: false,
            hits: m.hits.len(),
            hits_fmt: hits_badge(m.hits.len()),
            body: StepBody::Thinking(whole(m)),
        },
        StepKind::Text(m) => StepView {
            kind: TickKind::Text,
            time,
            label: "text".into(),
            summary: opening(&m.text).into(),
            meta: SharedString::default(),
            error: false,
            hits: m.hits.len(),
            hits_fmt: hits_badge(m.hits.len()),
            body: StepBody::Text(folded(m, REPLY_LINES)),
        },
    }
}

fn tool_view(call: &ToolCall, time: SharedString, paths: &Paths) -> StepView {
    let input: Vec<InputLine> = call.input.iter().map(input_line).collect();
    let output = call.result.as_ref().map(output_folded);
    let hits = input.iter().map(|l| l.hits.len()).sum::<usize>() + output.as_ref().map_or(0, |o| o.hits.len());
    StepView {
        kind: TickKind::Tool,
        time,
        label: call.name.clone().into(),
        summary: tool_summary(call, paths).into(),
        meta: call.result.as_ref().map_or_else(String::new, |r| result_label(&call.name, r)).into(),
        error: call.result.as_ref().is_some_and(|r| r.is_error),
        hits,
        hits_fmt: hits_badge(hits),
        body: StepBody::Tool { input, output },
    }
}

/// The argument as the collapsed row shows it: file paths shortened, Grep's path part too.
fn tool_summary(call: &ToolCall, paths: &Paths) -> String {
    match call.name.as_str() {
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => paths.short(&call.arg),
        "Grep" => {
            let path = call.input.iter().find(|l| matches!(&l.kind, ToolLineKind::Field(k) if k == "path")).map(|l| l.text.text.as_str());
            match path.and_then(|p| Some((call.arg.strip_suffix(p)?, p))) {
                Some((pattern, path)) if !pattern.is_empty() => format!("{pattern}{}", paths.short(path)),
                _ => call.arg.clone(),
            }
        }
        _ => call.arg.clone(),
    }
}

/// `212 lines`, `11 lines · 640 B`, `ok` for a successful edit, `error · 46 lines`.
fn result_label(name: &str, r: &ToolResult) -> String {
    if r.is_error {
        return if r.lines == 0 { "error".into() } else { format!("error · {}", count(r.lines, "line", "lines")) };
    }
    if matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
        return "ok".into();
    }
    if r.bytes == 0 {
        return String::new();
    }
    let lines = count(r.lines, "line", "lines");
    if r.bytes >= SIZE_SHOWN_BYTES { format!("{lines} · {}", size_label(r.bytes as u64)) } else { lines }
}

fn input_line(line: &ToolLine) -> InputLine {
    let (prefix, kind) = match &line.kind {
        ToolLineKind::Description => ("# ".to_string(), InputKind::Description),
        ToolLineKind::Command => ("$ ".to_string(), InputKind::Command),
        ToolLineKind::Field(key) => (format!("{key}: "), InputKind::Field),
    };
    let shift = prefix.len();
    InputLine {
        kind,
        hits: line.text.ranges.iter().map(|r| r.start + shift..r.end + shift).collect(),
        text: (prefix + &line.text.text).into(),
    }
}

/// A tool output folded at `OUTPUT_LINES`; the link under it says what is left and how big the
/// output is: `+ 40 more lines · 4.1 KB`.
fn output_folded(r: &ToolResult) -> Folded {
    let m = &r.output;
    let fold = fold(&m.text, &m.hits, OUTPUT_LINES, FOLD_CHARS);
    let more = fold.map_or_else(String::new, |at| {
        let shown = m.text[..at].lines().count();
        let size = size_label(r.bytes as u64);
        if r.lines > shown {
            format!("+ {} more lines · {size}", r.lines - shown)
        } else {
            format!("+ {} more · {size}", chars_label(m.chars.saturating_sub(m.text[..at].chars().count())))
        }
    });
    Folded::new(&m.text, m.hits.clone(), fold, more.into())
}

/// A prompt, reply or process text folded at `max_lines`.
fn folded(m: &Marked, max_lines: usize) -> Folded {
    let fold = fold(&m.text, &m.hits, max_lines, FOLD_CHARS);
    Folded::new(&m.text, m.hits.clone(), fold, show_all(fold, m.lines, m.chars, max_lines))
}

fn plain_folded(text: &str, max_lines: usize) -> Folded {
    let fold = fold(text, &[], max_lines, FOLD_CHARS);
    Folded::new(text, Vec::new(), fold, show_all(fold, text.lines().count(), text.chars().count(), max_lines))
}

fn whole(m: &Marked) -> Folded {
    Folded::new(&m.text, m.hits.clone(), None, SharedString::default())
}

/// `Show all · 18 lines`, or `Show all · 2.0k chars` when the fold cut one long line.
fn show_all(fold: Option<usize>, lines: usize, chars: usize, max_lines: usize) -> SharedString {
    match fold {
        None => SharedString::default(),
        Some(_) if lines > max_lines => format!("Show all · {lines} lines").into(),
        Some(_) => format!("Show all · {}", chars_label(chars)).into(),
    }
}

/// The first non-empty line, cut short; `…` when there is more.
fn opening(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().unwrap_or("");
    let cut: String = first.chars().take(OPENING_CHARS).collect();
    if cut.len() < first.len() || lines.next().is_some() { format!("{cut}…") } else { cut }
}

fn hits_badge(hits: usize) -> SharedString {
    if hits == 0 { SharedString::default() } else { hits.to_string().into() }
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// `45s`, `1m 02s`, `6m 30s`, `1h 12m`; nothing for no time at all.
fn duration_label(ms: i64) -> Option<String> {
    if ms <= 0 {
        return None;
    }
    let s = (ms + 500) / 1000;
    Some(match s {
        0 => "<1s".to_string(),
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {:02}s", s / 60, s % 60),
        s => format!("{}h {:02}m", s / 3600, s % 3600 / 60),
    })
}

/// `640 B`, `4.1 KB`, `860 KB`, `1.2 MB`.
fn size_label(bytes: u64) -> String {
    let kb = bytes as f64 / 1024.0;
    match bytes {
        0..1024 => format!("{bytes} B"),
        _ if kb < 10.0 => format!("{kb:.1} KB"),
        _ if kb < 1024.0 => format!("{kb:.0} KB"),
        _ => format!("{:.1} MB", kb / 1024.0),
    }
}

/// `880 chars`, `1.2k chars`, `21k chars`.
fn chars_label(n: usize) -> String {
    match n {
        0..1000 => format!("{n} chars"),
        1000..10_000 => format!("{:.1}k chars", n as f64 / 1000.0),
        _ => format!("{}k chars", (n + 500) / 1000),
    }
}

fn local(ts: Option<i64>) -> Option<DateTime<Local>> {
    Local.timestamp_millis_opt(ts?).single()
}

fn date_of(ts: Option<i64>) -> Option<NaiveDate> {
    local(ts).map(|t| t.date_naive())
}

/// `HH:MM` on `day`, `MM-DD HH:MM` on any other; empty without a timestamp.
fn time_label(ts: Option<i64>, day: NaiveDate) -> SharedString {
    match local(ts) {
        Some(t) if t.date_naive() == day => t.format("%H:%M").to_string().into(),
        Some(t) => t.format("%m-%d %H:%M").to_string().into(),
        None => SharedString::default(),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use agent_session_core::claude::conversation::read_conversation;
    use agent_session_core::model::{Attachment, Reply};
    use agent_session_core::search::{Highlighted, Matcher};

    const PROJECT: &str = "/Users/me/Code/alpha";

    pub fn at(month: u32, day: u32, h: u32, m: u32, s: u32) -> Option<i64> {
        Local.with_ymd_and_hms(2026, month, day, h, m, s).single().map(|t| t.timestamp_millis())
    }

    pub fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 17).unwrap()
    }

    pub fn text(t: &str) -> Marked {
        Marked::new(t, usize::MAX, None)
    }

    pub fn marked(t: &str, needle: &str) -> Marked {
        Marked::new(t, usize::MAX, Some(&Matcher::regex(needle, false)))
    }

    pub fn turn(number: Option<u32>, ts: Option<i64>, prompt: &str, steps: Vec<Step>) -> Turn {
        Turn {
            number,
            origin: None,
            ts,
            prompt: text(prompt),
            attachments: Vec::new(),
            steps,
            reply: Some(Reply { ts, text: text("done") }),
            end: TurnEnd::Completed,
            duration_ms: None,
            last_ts: ts,
        }
    }

    pub fn tool(name: &str, arg: &str, result: Option<(&str, bool)>) -> Step {
        let result = result.map(|(out, is_error)| {
            let output = text(out);
            ToolResult { bytes: out.len(), lines: output.lines, output, is_error }
        });
        Step { ts: None, kind: StepKind::Tool(ToolCall { name: name.into(), arg: arg.into(), input: Vec::new(), result }) }
    }

    pub fn says(t: &str) -> Step {
        Step { ts: None, kind: StepKind::Text(text(t)) }
    }

    pub fn thinks(t: &str) -> Step {
        Step { ts: None, kind: StepKind::Thinking(text(t)) }
    }

    pub fn conv(items: Vec<ConvItem>) -> Conversation {
        Conversation { items, bytes: 0, skipped_lines: 0 }
    }

    pub fn view_of(items: Vec<ConvItem>) -> ConversationView {
        build(&conv(items), PROJECT, today())
    }

    pub fn turn_card(view: &ConversationView, ix: usize) -> &TurnCard {
        let Card::Turn(card) = &view.cards[ix] else { panic!("card {ix} is not a turn") };
        card
    }

    pub fn lines(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n")
    }

    /// Not a behavior test: how long reading a real transcript, building its cards and the row
    /// table take. `ASM_BENCH_TRANSCRIPTS=<path>,<path> cargo test --release -p agent-session-manager
    /// real_transcript_timings -- --ignored --nocapture`; `ASM_BENCH_REGEX` reads with a search,
    /// `ASM_BENCH_PROJECT` is the project directory paths are shortened against, and
    /// `ASM_BENCH_DUMP=1` prints every card's strings.
    #[test]
    #[ignore]
    fn real_transcript_timings() {
        let paths = std::env::var("ASM_BENCH_TRANSCRIPTS").expect("ASM_BENCH_TRANSCRIPTS lists transcript paths");
        let matcher = std::env::var("ASM_BENCH_REGEX").ok().map(|re| Matcher::regex(&re, false));
        let project = std::env::var("ASM_BENCH_PROJECT").unwrap_or_default();
        let today = Local::now().date_naive();
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        for path in paths.split(',') {
            read_conversation(path.as_ref(), matcher.as_ref()).unwrap(); // page cache
            let mut runs = Vec::new();
            let mut view = ConversationView::default();
            for _ in 0..5 {
                let t = std::time::Instant::now();
                let conv = read_conversation(path.as_ref(), matcher.as_ref()).unwrap();
                let read = t.elapsed();
                let t = std::time::Instant::now();
                view = build(&conv, &project, today);
                let built = t.elapsed();
                let t = std::time::Instant::now();
                let closed = rows(&view, &HashSet::new());
                runs.push((ms(read), ms(built), ms(t.elapsed()), closed.len()));
            }
            let all: HashSet<usize> = (0..view.cards.len()).collect();
            let t = std::time::Instant::now();
            let open = rows(&view, &all).len();
            let open_ms = ms(t.elapsed());
            let steps: usize = view.cards.iter().map(|c| if let Card::Turn(t) = c { t.steps.len() } else { 0 }).sum();
            let mut sorted: Vec<f64> = runs.iter().map(|r| r.1 + r.2).collect();
            sorted.sort_by(f64::total_cmp);
            let name = std::path::Path::new(path).file_stem().unwrap().to_string_lossy();
            println!("{}: {} B, {} cards ({}), {} steps, {} hit places", &name[..8.min(name.len())], view.bytes, view.cards.len(), view.summary, steps, view.hits.len());
            println!("  read / build / rows(closed, {} rows) ms: {}", runs[0].3, runs.iter().map(|r| format!("{:.2}/{:.2}/{:.3}", r.0, r.1, r.2)).collect::<Vec<_>>().join("  "));
            println!("  build + rows median {:.2} ms; rows with every turn open: {open} rows in {open_ms:.3} ms", sorted[sorted.len() / 2]);
            if std::env::var_os("ASM_BENCH_DUMP").is_some() {
                dump(&view);
            }
        }
    }

    fn dump(view: &ConversationView) {
        for card in &view.cards {
            match card {
                Card::Compact(c) => println!("  ── {} · {}", c.label, c.summary_label),
                Card::Turn(t) => {
                    println!("  [{:?}] {} {} {} | {} | {} | {} | {}", t.status, t.label, t.origin, t.time, t.meta, t.errors, t.end_note, t.hits_fmt);
                    let prompt = t.prompt.fold.map_or(t.prompt.text.as_ref(), |at| &t.prompt.text[..at]);
                    println!("    prompt: {:?} {}", prompt.chars().take(100).collect::<String>(), t.prompt.more);
                    if !t.attachments.is_empty() {
                        println!("    attachments: {:?}", t.attachments);
                    }
                    if !t.stats.is_empty() {
                        println!("    stats: {}", t.stats);
                    }
                    for step in &t.steps {
                        println!("    {:>11} {:<12} {:<70} {} {}", step.time, step.label, step.summary.chars().take(70).collect::<String>(), step.meta, step.hits_fmt);
                        if let StepBody::Tool { output: Some(out), .. } = &step.body {
                            if !out.more.is_empty() {
                                println!("{:>30}{}", "", out.more);
                            }
                        }
                    }
                    match &t.tail {
                        Tail::Reply { time, text } => println!("    ASST {time}: {:?} {}", text.text.chars().take(80).collect::<String>(), text.more),
                        Tail::Interrupted { time } => println!("    {time} [Request interrupted by user]"),
                        Tail::ApiError { time, text } => println!("    {time} {:?}", text.text.chars().take(80).collect::<String>()),
                        Tail::Empty => {}
                    }
                }
            }
        }
    }

    #[test]
    fn the_summary_counts_turns_auto_segments_and_tool_calls() {
        let auto = Turn { origin: Some("task-notification".into()), ..turn(None, None, "Background command done", vec![tool("Bash", "ls", None)]) };
        let view = view_of(vec![
            ConvItem::Turn(turn(Some(1), None, "a", vec![tool("Read", "x", None), says("hm"), tool("Bash", "y", None)])),
            ConvItem::Turn(auto),
            ConvItem::Compact(Compaction { ts: None, summary: None }),
            ConvItem::Turn(turn(Some(2), None, "b", vec![])),
        ]);
        assert_eq!(view.summary, "2 turns · 1 auto · 3 tool calls");
        assert_eq!(view.cards.len(), 4, "one card per item, the divider included");

        let one = view_of(vec![ConvItem::Turn(turn(Some(1), None, "a", vec![tool("Read", "x", None)]))]);
        assert_eq!(one.summary, "1 turn · 1 tool call", "no auto part without AUTO cards");

        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../core/tests/fixtures/conversation.jsonl");
        let fixture = build(&read_conversation(path.as_ref(), None).unwrap(), PROJECT, today());
        assert_eq!(fixture.summary, "2 turns · 2 auto · 3 tool calls");
        assert!(fixture.bytes > 0);
    }

    #[test]
    fn a_card_shows_steps_duration_errors_and_how_the_turn_ended() {
        let start = at(10, 17, 10, 24, 0);
        let done = Turn {
            duration_ms: Some(390_000),
            ..turn(Some(3), start, "go", vec![tool("Bash", "cargo test", Some(("boom", true))), says("fixing"), tool("Edit", "x.rs", Some(("ok", false)))])
        };
        let interrupted = Turn { end: TurnEnd::Interrupted { ts: at(10, 17, 10, 30, 0) }, reply: None, last_ts: at(10, 17, 10, 25, 2), ..turn(Some(4), at(10, 17, 10, 24, 0), "and", vec![tool("Read", "a", None)]) };
        let failed = Turn {
            end: TurnEnd::ApiError { ts: start, text: "API Error: 529 overloaded\nretry later\nthird line".into() },
            reply: None,
            last_ts: at(10, 17, 10, 24, 45),
            ..turn(Some(5), start, "try", vec![])
        };
        let silent = Turn { end: TurnEnd::NoReply, reply: None, last_ts: at(10, 17, 11, 36, 30), ..turn(Some(6), start, "go on", vec![thinks("hmm")]) };
        let auto = Turn { origin: Some("task-notification".into()), ..turn(None, start, "Background command “pnpm build” completed", vec![]) };
        let view = view_of(vec![ConvItem::Turn(done), ConvItem::Turn(interrupted), ConvItem::Turn(failed), ConvItem::Turn(silent), ConvItem::Turn(auto)]);

        let done = turn_card(&view, 0);
        assert_eq!((done.label.as_ref(), done.auto, done.origin.as_ref()), ("TURN 3", false, ""));
        assert_eq!((done.meta.as_ref(), done.errors.as_ref(), done.end_note.as_ref(), done.status), ("3 steps · 6m 30s", "1 error", "", Status::Ok));
        assert!(matches!(&done.tail, Tail::Reply { text, .. } if text.text == "done"));

        let interrupted = turn_card(&view, 1);
        assert_eq!((interrupted.meta.as_ref(), interrupted.end_note.as_ref(), interrupted.status), ("1 step · 1m 02s", "interrupted", Status::Failed), "no duration record: last_ts - ts");
        assert!(matches!(&interrupted.tail, Tail::Interrupted { time } if time == "10:30"));

        let failed = turn_card(&view, 2);
        assert_eq!((failed.meta.as_ref(), failed.errors.as_ref(), failed.end_note.as_ref(), failed.status), ("45s", "", "API error", Status::Failed));
        let Tail::ApiError { text, .. } = &failed.tail else { panic!("expected an API error tail") };
        assert_eq!((&text.text[..text.fold.unwrap()], text.more.as_ref()), ("API Error: 529 overloaded\nretry later", "Show all · 3 lines"));

        let silent = turn_card(&view, 3);
        assert_eq!((silent.meta.as_ref(), silent.end_note.as_ref(), silent.status), ("1 step · 1h 12m", "no reply", Status::Off));
        assert!(matches!(silent.tail, Tail::Empty));

        let auto = turn_card(&view, 4);
        assert_eq!((auto.label.as_ref(), auto.auto, auto.origin.as_ref()), ("AUTO", true, "task-notification"));
        assert_eq!((auto.meta.as_ref(), auto.stats.as_ref()), ("", ""), "no steps, no duration: nothing to say");
        assert!(auto.ticks.is_empty() && auto.steps.is_empty());
    }

    #[test]
    fn stats_list_tools_by_count_then_text_and_thinking() {
        let steps = vec![
            thinks("plan"),
            tool("Read", "a", None),
            tool("Bash", "x", None),
            tool("Edit", "a", None),
            says("next"),
            tool("Bash", "y", None),
            tool("Read", "b", None),
            tool("Bash", "z", None),
            tool("Edit", "b", None),
            says("done?"),
            tool("Grep", "q", None),
        ];
        let view = view_of(vec![ConvItem::Turn(turn(Some(1), None, "go", steps))]);
        assert_eq!(turn_card(&view, 0).stats, "Bash 3 · Edit 2 · Read 2 · Grep 1 · text 2 · thinking 1", "ties go by name");
    }

    #[test]
    fn ticks_follow_the_steps_and_mark_errors_and_hits() {
        let hit = Step {
            ts: None,
            kind: StepKind::Tool(ToolCall {
                name: "Grep".into(),
                arg: "api_usage".into(),
                input: vec![ToolLine { kind: ToolLineKind::Field("pattern".into()), text: Highlighted { text: "api_usage".into(), ranges: vec![0..9] } }],
                result: Some(ToolResult { output: marked("schema.sql: api_usage\ndb.rs: api_usage", "api_usage"), bytes: 37, lines: 2, is_error: false }),
            }),
        };
        let steps = vec![thinks("let me see"), tool("Read", "a", Some(("x", false))), tool("Bash", "b", Some(("no", true))), says("found it"), hit];
        let view = view_of(vec![ConvItem::Turn(turn(Some(1), None, "go", steps))]);
        let card = turn_card(&view, 0);
        let kinds: Vec<TickKind> = card.ticks.iter().map(|t| t.kind).collect();
        assert_eq!(kinds, [TickKind::Thinking, TickKind::Tool, TickKind::Tool, TickKind::Text, TickKind::Tool]);
        assert_eq!(card.ticks.iter().map(|t| t.error).collect::<Vec<_>>(), [false, false, true, false, false]);
        assert_eq!(card.ticks.iter().map(|t| t.hit).collect::<Vec<_>>(), [false, false, false, false, true]);
        assert_eq!((card.steps[4].hits, card.steps[4].hits_fmt.as_ref()), (3, "3"), "input and output hits together");
        assert_eq!((card.hits, card.hits_fmt.as_ref()), (1, "1 hit"), "a card counts hit places, a step is one place");
        assert_eq!(view.hits, [HitPlace { item: 0, part: Part::Step(4) }]);
    }

    #[test]
    fn thinking_and_process_text_show_their_opening_and_thinking_its_length() {
        let long = format!("先看 toolbar.rs 里 Updated 下拉现在怎么建菜单\n{}", "再看 RangePreset 的边界。".repeat(60));
        let view = view_of(vec![ConvItem::Turn(turn(Some(1), at(10, 17, 10, 24, 0), "go", vec![thinks(&long), says("RangePreset 现在只有天数预设。")]))]);
        let card = turn_card(&view, 0);
        let thinking = &card.steps[0];
        assert_eq!((thinking.label.as_ref(), thinking.summary.as_ref()), ("thinking", "先看 toolbar.rs 里 Updated 下拉现在怎么建菜单…"));
        assert_eq!(thinking.meta, "1.2k chars");
        assert!(matches!(&thinking.body, StepBody::Thinking(f) if f.fold.is_none() && f.text.len() == long.len()), "open, a thinking shows whole");
        let text = &card.steps[1];
        assert_eq!((text.label.as_ref(), text.summary.as_ref(), text.meta.as_ref()), ("text", "RangePreset 现在只有天数预设。", ""));
        assert!(matches!(&text.body, StepBody::Text(f) if f.text == "RangePreset 现在只有天数预设。"));
    }

    #[test]
    fn a_tool_step_summarizes_its_argument_and_result() {
        let home = home_dir().to_string_lossy().into_owned();
        let deep = format!("{home}/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-component-0.6.1/src/date_picker.rs");
        let bash_out = lines(11) + &"x".repeat(562);
        let read_out = "a\n".repeat(212);
        let failed = lines(46);
        let grep = ToolCall {
            name: "Grep".into(),
            arg: format!("RangePreset {PROJECT}/crates/app/src"),
            input: vec![
                ToolLine { kind: ToolLineKind::Field("pattern".into()), text: Highlighted::plain("RangePreset") },
                ToolLine { kind: ToolLineKind::Field("path".into()), text: Highlighted::plain(format!("{PROJECT}/crates/app/src")) },
            ],
            result: None,
        };
        let bash = ToolCall {
            name: "Bash".into(),
            arg: "cargo test -p agent-session-manager".into(),
            input: vec![
                ToolLine { kind: ToolLineKind::Description, text: Highlighted::plain("Run the tests") },
                ToolLine { kind: ToolLineKind::Command, text: Highlighted { text: "cargo test -p agent-session-manager".into(), ranges: vec![0..5] } },
                ToolLine { kind: ToolLineKind::Field("timeout".into()), text: Highlighted::plain("60000") },
            ],
            result: None,
        };
        let steps = vec![
            Step { ts: None, kind: StepKind::Tool(bash) },
            tool("Bash", "cargo build", Some((&bash_out, false))),
            tool("Read", &format!("{PROJECT}/crates/app/src/toolbar.rs"), Some((&read_out, false))),
            tool("Read", &format!("{home}/notes/todo.md"), None),
            tool("Read", &deep, None),
            tool("Edit", &format!("{PROJECT}/crates/app/src/model.rs"), Some(("The file has been updated.", false))),
            tool("Bash", "cargo test -p agent-session-manager keyboard_tests", Some((&failed, true))),
            Step { ts: None, kind: StepKind::Tool(grep) },
            tool("Read", "/opt/elsewhere/file.txt", None),
            tool("Read", "/opt/homebrew/Cellar/python@3.13/3.13.1/Frameworks/Python.framework/Versions/3.13/lib/site.py", None),
        ];
        let view = view_of(vec![ConvItem::Turn(turn(Some(1), None, "go", steps))]);
        let card = turn_card(&view, 0);
        let row = |ix: usize| (card.steps[ix].label.as_ref(), card.steps[ix].summary.as_ref(), card.steps[ix].meta.as_ref());

        assert_eq!(row(0), ("Bash", "cargo test -p agent-session-manager", ""), "no result yet: nothing after the argument");
        let StepBody::Tool { input, output } = &card.steps[0].body else { panic!("expected a tool body") };
        let shown: Vec<&str> = input.iter().map(|l| l.text.as_ref()).collect();
        assert_eq!(shown, ["# Run the tests", "$ cargo test -p agent-session-manager", "timeout: 60000"]);
        assert_eq!(input.iter().map(|l| l.kind).collect::<Vec<_>>(), [InputKind::Description, InputKind::Command, InputKind::Field]);
        assert_eq!(&input[1].text[input[1].hits[0].clone()], "cargo", "hits move with the prefix");
        assert!(output.is_none());

        assert_eq!(row(1).2, "11 lines · 640 B");
        let StepBody::Tool { output: Some(out), .. } = &card.steps[1].body else { panic!("expected an output") };
        assert!(out.fold.is_none(), "11 lines fit in the output box");
        assert_eq!(row(2), ("Read", "crates/app/src/toolbar.rs", "212 lines"), "inside the project: relative; under 512 B: no size");
        assert_eq!(row(3).1, "~/notes/todo.md");
        assert_eq!(row(4).1, "~/.cargo/registry/…/gpui-component-0.6.1/src/date_picker.rs", "long paths keep both ends");
        assert_eq!(row(5).2, "ok");
        assert_eq!(row(6).2, "error · 46 lines");
        assert!(card.steps[6].error && !card.steps[5].error);
        let StepBody::Tool { output: Some(out), .. } = &card.steps[6].body else { panic!("expected an output") };
        assert_eq!((out.text[..out.fold.unwrap()].lines().count(), out.more.to_string()), (OUTPUT_LINES, format!("+ 34 more lines · {} B", failed.len())));
        assert_eq!(row(7).1, "RangePreset crates/app/src", "Grep: the path part is shortened too");
        assert_eq!(row(8).1, "/opt/elsewhere/file.txt");
        assert_eq!(row(9).1, "/opt/homebrew/Cellar/…/Versions/3.13/lib/site.py", "an absolute path keeps its root");
    }

    #[test]
    fn folding_counts_source_lines_and_caps_one_long_line() {
        let five = lines(5);
        let at = fold(&five, &[], 4, FOLD_CHARS).unwrap();
        assert_eq!(&five[..at], "line 1\nline 2\nline 3\nline 4");
        assert_eq!(fold(&lines(4), &[], 4, FOLD_CHARS), None);
        assert_eq!(fold(&format!("{}\n\n", lines(4)), &[], 4, FOLD_CHARS), None, "trailing blank lines are not worth a fold");

        let wide = "很长的一行".repeat(400);
        let at = fold(&wide, &[], 4, FOLD_CHARS).unwrap();
        assert_eq!(wide[..at].chars().count(), FOLD_CHARS, "one long line folds by characters, on a char boundary");
        assert_eq!(fold(&"x".repeat(FOLD_CHARS), &[], 4, FOLD_CHARS), None);

        let prompt = format!("{}\n{}", "a long first line that wraps on screen ".repeat(10), lines(17));
        let view = view_of(vec![ConvItem::Turn(Turn { reply: Some(Reply { ts: None, text: text(&lines(14)) }), ..turn(Some(1), None, &prompt, vec![]) })]);
        let card = turn_card(&view, 0);
        assert_eq!(card.prompt.text[..card.prompt.fold.unwrap()].lines().count(), PROMPT_LINES);
        assert_eq!(card.prompt.more, "Show all · 18 lines");
        let Tail::Reply { text: reply, .. } = &card.tail else { panic!("expected a reply") };
        assert_eq!((reply.text[..reply.fold.unwrap()].lines().count(), reply.more.as_ref()), (REPLY_LINES, "Show all · 14 lines"));

        let paragraphs = "one\ntwo\nthree\nfour\nfive\n\nsix\nseven";
        let view = view_of(vec![ConvItem::Turn(Turn { reply: Some(Reply { ts: None, text: text(paragraphs) }), ..turn(Some(1), None, "p", vec![]) })]);
        let Tail::Reply { text: reply, .. } = &turn_card(&view, 0).tail else { panic!("expected a reply") };
        assert_eq!(reply.short, "one\ntwo\nthree\nfour\nfive", "a fold after a blank line leaves no empty line above the link");
        assert_eq!(reply.text, paragraphs);

        let view = view_of(vec![ConvItem::Turn(turn(Some(1), None, &wide, vec![]))]);
        assert_eq!(turn_card(&view, 0).prompt.more, "Show all · 2.0k chars", "a fold by characters counts characters");
        let short = view_of(vec![ConvItem::Turn(turn(Some(1), None, "short", vec![]))]);
        assert_eq!((turn_card(&short, 0).prompt.fold, turn_card(&short, 0).prompt.more.as_ref()), (None, ""));
    }

    #[test]
    fn a_hit_inside_a_fold_opens_it() {
        let ten = lines(10);
        let late = ten.find("line 9").unwrap();
        assert_eq!(fold(&ten, &[late..late + 6], 6, FOLD_CHARS), None, "a hit past the fold shows the whole text");
        assert!(fold(&ten, &[0..6], 6, FOLD_CHARS).is_some(), "a hit inside the folded view keeps the fold");

        let reply = marked(&lines(10), "line 9");
        let output = marked(&lines(40), "line 30");
        let call = ToolCall { name: "Bash".into(), arg: "x".into(), input: Vec::new(), result: Some(ToolResult { output, bytes: 300, lines: 40, is_error: false }) };
        let view = view_of(vec![ConvItem::Turn(Turn {
            reply: Some(Reply { ts: None, text: reply }),
            prompt: marked(&lines(8), "line 8"),
            ..turn(Some(1), None, "", vec![Step { ts: None, kind: StepKind::Tool(call) }])
        })]);
        let card = turn_card(&view, 0);
        assert_eq!((card.prompt.fold, card.prompt.more.as_ref()), (None, ""));
        assert!(matches!(&card.tail, Tail::Reply { text, .. } if text.fold.is_none()));
        assert!(matches!(&card.steps[0].body, StepBody::Tool { output: Some(out), .. } if out.fold.is_none() && out.more.is_empty()));
    }

    #[test]
    fn times_carry_the_date_when_the_day_changes() {
        let mut first = turn(Some(7), at(10, 17, 21, 40, 0), "a", vec![Step { ts: at(10, 17, 21, 41, 0), kind: StepKind::Text(text("x")) }]);
        first.reply = Some(Reply { ts: at(10, 17, 21, 49, 0), text: text("y") });
        let items = vec![
            ConvItem::Turn(first),
            ConvItem::Compact(Compaction { ts: at(10, 17, 22, 5, 0), summary: Some(text(&"s".repeat(9_800))) }),
            ConvItem::Turn(turn(None, at(10, 17, 22, 6, 0), "auto", vec![])),
            ConvItem::Turn(turn(Some(8), None, "no time", vec![])),
            ConvItem::Turn(turn(Some(9), at(10, 18, 9, 2, 0), "next day", vec![Step { ts: at(10, 18, 9, 3, 0), kind: StepKind::Text(text("z")) }])),
        ];
        let label = |view: &ConversationView, ix: usize| match &view.cards[ix] {
            Card::Turn(t) => t.time.to_string(),
            Card::Compact(c) => c.label.to_string(),
        };

        let view = build(&conv(items), PROJECT, NaiveDate::from_ymd_opt(2026, 10, 18).unwrap());
        assert_eq!(label(&view, 0), "10-17 21:40", "the first card is not from today");
        assert_eq!(label(&view, 1), "context compacted · 22:05");
        assert_eq!(label(&view, 2), "22:06");
        assert_eq!(label(&view, 3), "");
        assert_eq!(label(&view, 4), "10-18 09:02");
        let first = turn_card(&view, 0);
        assert_eq!(first.steps[0].time, "21:41", "steps and replies only say the time on their card's day");
        assert!(matches!(&first.tail, Tail::Reply { time, .. } if time == "21:49"));
        assert_eq!(turn_card(&view, 4).steps[0].time, "09:03");
        let Card::Compact(compact) = &view.cards[1] else { panic!("expected the divider") };
        assert_eq!(compact.summary_label, "summary · 9.8k chars");

        let items = vec![ConvItem::Turn(turn(Some(1), at(10, 17, 21, 40, 0), "a", vec![]))];
        assert_eq!(label(&build(&conv(items), PROJECT, today()), 0), "21:40", "today's first card has no date");
    }

    #[test]
    fn attachments_are_labeled_with_their_kind_and_size() {
        let pasted = Turn {
            attachments: vec![Attachment { kind: "image".into(), bytes: 1_258_291 }, Attachment { kind: "document".into(), bytes: 880_640 }],
            ..turn(Some(1), None, "see the screenshot", vec![])
        };
        let view = view_of(vec![ConvItem::Turn(pasted)]);
        assert_eq!(turn_card(&view, 0).attachments, ["image · 1.2 MB", "document · 860 KB"]);
    }

    #[test]
    fn a_closed_turn_is_a_head_and_a_tail_and_an_open_turn_lists_its_steps_between() {
        let view = view_of(vec![
            ConvItem::Turn(turn(Some(1), None, "a", vec![tool("Read", "x", None), says("y")])),
            ConvItem::Compact(Compaction { ts: None, summary: None }),
            ConvItem::Turn(turn(Some(2), None, "b", vec![])),
        ]);
        assert_eq!(rows(&view, &HashSet::new()), [Row::Head(0), Row::Tail(0), Row::Compact(1), Row::Head(2), Row::Tail(2)]);
        assert_eq!(
            rows(&view, &HashSet::from([0, 2])),
            [Row::Head(0), Row::Step(0, 0), Row::Step(0, 1), Row::Tail(0), Row::Compact(1), Row::Head(2), Row::Tail(2)],
            "a turn without steps has nothing to open"
        );
        assert_eq!(card_rows(0, &view.cards[0], true), [Row::Head(0), Row::Step(0, 0), Row::Step(0, 1), Row::Tail(0)], "what a splice puts in");
        let open = rows(&view, &HashSet::from([0]));
        assert_eq!(row_of(&open, &HitPlace { item: 0, part: Part::Step(1) }), Some(2));
        assert_eq!(row_of(&open, &HitPlace { item: 0, part: Part::Reply }), Some(3));
        assert_eq!(row_of(&rows(&view, &HashSet::new()), &HitPlace { item: 0, part: Part::Step(1) }), Some(0), "a closed turn's steps are under its head");
        assert_eq!(row_of(&open, &HitPlace { item: 1, part: Part::Summary }), Some(4));
    }
}
