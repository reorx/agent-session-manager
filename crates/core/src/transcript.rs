//! Compact text handoffs separated by XML-style tags.
//! No index or model call is needed. The GUI can call `read_transcript` off-thread.
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::claude::clean::clean_prompt;
use crate::claude::discover::discover_all;
use crate::claude::parse::{Content, parse_content, user_text};
use crate::claude::paths::{ClaudeDir, expand_user};
use crate::model::TranscriptExport;
use crate::search::query::Query;

mod reading;

/// What part of a transcript to export, and how (`asm transcript`'s reading options). The
/// default is the full export of `read_transcript`.
#[derive(Clone, Debug, Default)]
pub struct TranscriptOptions {
    /// One line per turn instead of the text.
    pub outline: bool,
    /// Only these turns.
    pub turns: Option<TurnRange>,
    /// Only the turns where any TERM occurs in the parts shown.
    pub grep: Option<Query>,
    /// With `grep`, also this many turns before and after each turn with a hit.
    pub context: u32,
    /// Show the agent's thinking.
    pub thinking: bool,
    /// Show each tool call with its input and output.
    pub tools: bool,
}

impl TranscriptOptions {
    /// No reading option: the full export, unchanged.
    pub fn is_full(&self) -> bool {
        !self.outline && self.turns.is_none() && self.grep.is_none() && !self.thinking && !self.tools
    }
}

/// `--turns`: `N`, `N-M` or `N-` (to the last turn); turns start at 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurnRange {
    pub first: u32,
    pub last: Option<u32>,
}

impl TurnRange {
    pub fn parse(s: &str) -> Result<TurnRange, String> {
        let s = s.trim();
        let number = |t: &str| match t.parse::<u32>() {
            Ok(0) => Err("turns start at 1".to_string()),
            Ok(n) => Ok(n),
            Err(_) => Err(format!("'{s}' is not a turn range (N, N-M or N-)")),
        };
        match s.split_once('-') {
            None => number(s).map(|n| TurnRange { first: n, last: Some(n) }),
            Some((first, "")) => Ok(TurnRange { first: number(first)?, last: None }),
            Some((first, last)) => {
                let (first, last) = (number(first)?, number(last)?);
                if first > last {
                    return Err(format!("'{s}' ends before it starts"));
                }
                Ok(TurnRange { first, last: Some(last) })
            }
        }
    }

    pub fn contains(&self, turn: u32) -> bool {
        turn >= self.first && self.last.is_none_or(|last| turn <= last)
    }
}

/// The export `opts` asks for: `read_transcript` without reading options, else the turns asked
/// for with every item `asm search` searches in them (see `reading`).
pub fn read_transcript_with(path: &Path, opts: &TranscriptOptions) -> Result<TranscriptExport> {
    if opts.is_full() { read_transcript(path) } else { reading::read(path, opts) }
}

/// Resolve a local file, exact session ID, or unambiguous ID prefix.
/// Discovery is read-only; history-only sessions have no transcript to export.
pub fn resolve_transcript(claude: &ClaudeDir, favorites: Option<&ClaudeDir>, input: &str) -> Result<PathBuf> {
    let path = expand_user(input);
    if path.is_file() {
        return Ok(path);
    }
    if input.contains('/') || input.ends_with(".jsonl") {
        bail!("transcript file not found: {}", path.display());
    }
    if input.is_empty() {
        bail!("transcript ID must not be empty");
    }
    let files = discover_all(claude, favorites)?.sessions;
    let exact: Vec<_> = files.iter().filter(|f| f.session_id == input).collect();
    let hits = if exact.is_empty() {
        files.iter().filter(|f| f.session_id.starts_with(input)).collect()
    } else {
        exact
    };
    match hits.as_slice() {
        [file] => Ok(file.path.clone()),
        [] => bail!("no transcript matches '{input}' in {} (history-only sessions cannot be exported)", claude.projects_dir().display()),
        _ => bail!("ambiguous transcript ID '{input}'; use a full ID or file path:\n{}", hits.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>().join("\n")),
    }
}

#[derive(Deserialize)]
pub(crate) struct Record<'a> {
    #[serde(rename = "type")]
    pub(crate) kind: Option<String>,
    pub(crate) uuid: Option<String>,
    #[serde(rename = "parentUuid")]
    pub(crate) parent_uuid: Option<String>,
    #[serde(rename = "requestRef")]
    pub(crate) request_ref: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) id: Option<String>,
    #[serde(rename = "querySource")]
    pub(crate) query_source: Option<String>,
    #[serde(rename = "shapeHash")]
    pub(crate) shape_hash: Option<String>,
    pub(crate) keep: Option<u32>,
    pub(crate) tail: Option<Vec<serde::de::IgnoredAny>>,
    pub(crate) params: Option<RequestParams>,
    pub(crate) subtype: Option<String>,
    #[serde(rename = "isMeta", default)]
    pub(crate) is_meta: bool,
    #[serde(rename = "isSidechain", default)]
    pub(crate) is_sidechain: bool,
    #[serde(rename = "isCompactSummary", default)]
    pub(crate) is_compact_summary: bool,
    #[serde(rename = "isAbortedMidStream", default)]
    pub(crate) is_aborted: bool,
    pub(crate) timestamp: Option<String>,
    #[serde(rename = "isApiErrorMessage", default)]
    pub(crate) is_api_error: bool,
    #[serde(rename = "interruptedMessageId")]
    pub(crate) interrupted_message_id: Option<String>,
    /// Present on the record that hands over an accepted plan; only its presence is read.
    #[serde(rename = "planContent")]
    pub(crate) plan_content: Option<serde::de::IgnoredAny>,
    #[serde(rename = "durationMs")]
    pub(crate) duration_ms: Option<i64>,
    pub(crate) origin: Option<Origin>,
    #[serde(borrow)]
    pub(crate) message: Option<Message<'a>>,
}

#[derive(Deserialize)]
pub(crate) struct RequestParams {
    pub(crate) betas: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub(crate) struct Origin {
    pub(crate) kind: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct Message<'a> {
    pub(crate) id: Option<String>,
    pub(crate) stop_reason: Option<String>,
    #[serde(borrow)]
    pub(crate) content: Option<&'a RawValue>,
    pub(crate) model: Option<String>,
    #[serde(borrow)]
    pub(crate) usage: Option<crate::claude::parse::RawUsage<'a>>,
}

#[derive(Default)]
struct Turn {
    text: String,
    message_id: Option<String>,
    tools: BTreeMap<String, usize>,
    started: bool,
    completed: bool,
}

impl Turn {
    fn finish(&mut self, output: &mut String) {
        if !self.started {
            return;
        }
        if !self.tools.is_empty() {
            let calls = self.tools.iter().map(|(name, count)| {
                let times = if *count == 1 { "time" } else { "times" };
                format!("{name} {count} {times}")
            }).collect::<Vec<_>>().join(", ");
            output.push_str(&format!("<turn-summary>\nCalled {}.\n</turn-summary>\n", sanitize_text(&calls)));
        }
        if self.text.is_empty() {
            output.push_str("<agent status=\"missing\">\n[No final response recorded]\n</agent>\n\n");
        } else {
            write_body(output, "agent", &self.text);
        }
        *self = Self::default();
    }

    fn assistant(&mut self, message: &Message, content: &Content) {
        self.started = true;
        let mut has_tools = false;
        if let Content::Blocks(blocks) = content {
            for block in blocks {
                if block.kind.as_deref() == Some("tool_use") {
                    has_tools = true;
                    *self.tools.entry(block.name.as_deref().unwrap_or("unknown").to_owned()).or_default() += 1;
                }
            }
        }
        // Text accompanying a tool call is commentary, not the final response.
        if has_tools || message.stop_reason.as_deref() == Some("tool_use") {
            self.text.clear();
            self.message_id = None;
            return;
        }
        if let Some(text) = user_text(content).filter(|text| !text.trim().is_empty()) {
            // Claude can write multiple content blocks of one API message as separate
            // JSONL records. Preserve those blocks, but replace earlier API messages.
            if message.id.is_some() && message.id == self.message_id && !self.text.is_empty() {
                self.text.push_str("\n\n");
                self.text.push_str(text.trim());
            } else {
                self.text = text.trim().to_owned();
            }
            self.message_id = message.id.clone();
            self.completed = message.stop_reason.as_deref() == Some("end_turn");
        }
    }
}

/// Keep tabs and line breaks; replace unreadable controls and U+FFFE/U+FFFF visibly.
fn sanitize_text(text: &str) -> String {
    text.chars().map(|c| match c {
        '\u{9}' | '\u{a}' | '\u{d}' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}' => c,
        _ => '\u{fffd}',
    }).collect()
}

fn xml_attribute(text: &str) -> String {
    sanitize_text(text).replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        .replace('"', "&quot;").replace('\n', "&#10;").replace('\r', "&#13;").replace('\t', "&#9;")
}

fn write_body(output: &mut String, role: &str, text: &str) {
    let body = sanitize_text(text);
    output.push_str(&format!("<{role}>\n{body}\n</{role}>\n\n"));
}

/// The facts of a user record that decide whether the user typed it. The readers here and the
/// indexer's parser (`claude/parse.rs`) each fill it in from their own record type, so turn
/// numbers and the index's `prompt` rows follow one rule.
pub(crate) struct UserFlags<'a> {
    pub(crate) is_meta: bool,
    pub(crate) is_compact_summary: bool,
    /// `origin.kind`; absent on records older than the field.
    pub(crate) origin: Option<&'a str>,
    /// The record has an `interruptedMessageId`.
    pub(crate) interrupted: bool,
    /// The record has a `planContent`: Claude Code wrote it after a plan was accepted and the
    /// context cleared (`Implement the following plan:…`).
    pub(crate) plan: bool,
}

impl Record<'_> {
    pub(crate) fn user_flags(&self) -> UserFlags<'_> {
        UserFlags {
            is_meta: self.is_meta,
            is_compact_summary: self.is_compact_summary,
            origin: self.origin.as_ref().and_then(|o| o.kind.as_deref()),
            interrupted: self.interrupted_message_id.is_some(),
            plan: self.plan_content.is_some(),
        }
    }
}

/// A user record with text or media of its own, as opposed to tool results only. These are the
/// records that can open a turn, and the index keeps a row for each of them (a media-only one
/// with empty text), which is what lets an appending parser know the uuids already taken.
pub(crate) fn has_own_input(content: &Content) -> bool {
    match content {
        Content::Text(_) => true,
        Content::Blocks(blocks) => {
            blocks.iter().any(|b| b.kind.as_deref() == Some("text"))
                || blocks.iter().any(|b| matches!(b.kind.as_deref(), Some("image" | "document")))
                    && !blocks.iter().any(|b| b.kind.as_deref() == Some("tool_result"))
        }
        Content::Other => false,
    }
}

/// Which records of one transcript file the readers use. The detail panel, the TXT export and the
/// indexer's parser all go through it, so they leave out the same records and agree on which
/// user records open a turn.
///
/// A sidechain record of a main transcript is left out (a subagent's own file is all sidechain).
/// So is a record whose uuid an earlier record of the same group already had; the groups are the
/// user records with text or media of their own (`has_own_input`) and all other records. The
/// uuid is taken even when the record is left out.
#[derive(Debug, Default)]
pub(crate) struct RecordFilter {
    subagent: bool,
    user_input: HashSet<String>,
    other: HashSet<String>,
}

impl RecordFilter {
    pub(crate) fn for_file(path: &Path) -> Self {
        let subagent = path.parent().and_then(Path::file_name).is_some_and(|name| name == "subagents");
        RecordFilter { subagent, ..Self::default() }
    }

    /// Continue a main transcript whose user records with input of their own had these uuids.
    pub(crate) fn with_user_input(uuids: HashSet<String>) -> Self {
        RecordFilter { user_input: uuids, ..Self::default() }
    }

    /// Whether the readers use this record; its uuid is taken either way.
    pub(crate) fn keep(&mut self, uuid: Option<&str>, sidechain: bool, user_input: bool) -> bool {
        let seen = if user_input { &mut self.user_input } else { &mut self.other };
        let first = uuid.filter(|u| !u.is_empty()).is_none_or(|u| seen.insert(u.to_owned()));
        first && (self.subagent || !sidechain)
    }
}

impl Record<'_> {
    /// The record's content, parsed once for the filter and the reader.
    pub(crate) fn content(&self) -> Option<Content<'_>> {
        self.message.as_ref().and_then(|m| m.content).map(parse_content)
    }

    /// A user record with input of its own: the filter's first group.
    pub(crate) fn is_user_input(&self, content: Option<&Content>) -> bool {
        self.kind.as_deref() == Some("user") && content.is_some_and(has_own_input)
    }
}

/// Who wrote the record: the user, or Claude Code on its own (`origin.kind`). Of what Claude Code
/// writes by itself, only the plan it hands over after the context is cleared starts a turn: the
/// user asked for it by accepting the plan (brief decision 24). Usage-limit continuations, task
/// notifications and messages from other sessions do not.
fn typed_by_the_user(flags: &UserFlags) -> bool {
    match flags.origin {
        None | Some("human") => true,
        Some("auto-continuation") => flags.plan,
        Some(_) => false,
    }
}

/// Output Claude Code records after a local command (`/model`, `!ls`): the command's record
/// opens the turn, its output belongs to that turn (brief decision 25).
const LOCAL_OUTPUT_TAGS: [&str; 4] = ["<local-command-stdout>", "<local-command-stderr>", "<bash-stdout>", "<bash-stderr>"];

pub(crate) fn is_local_command_output(text: &str) -> bool {
    let text = text.trim_start();
    LOCAL_OUTPUT_TAGS.iter().any(|tag| text.starts_with(tag))
}

pub(crate) fn is_human_input(flags: &UserFlags, content: &Content) -> bool {
    if flags.is_meta || flags.is_compact_summary || !typed_by_the_user(flags) {
        return false;
    }
    if let Content::Blocks(blocks) = content {
        if blocks.iter().any(|b| b.kind.as_deref() == Some("tool_result")) {
            return false;
        }
    }
    let text = user_text(content).unwrap_or_default();
    if text.starts_with("[Image: source:") || is_local_command_output(&text) {
        return false;
    }
    !clean_prompt(&text).is_empty() || matches!(content, Content::Blocks(blocks)
        if blocks.iter().any(|b| matches!(b.kind.as_deref(), Some("image" | "document"))))
}

pub(crate) fn is_interruption(flags: &UserFlags, text: &str) -> bool {
    flags.interrupted || text.trim().starts_with("[Request interrupted by user")
}

/// A user record that starts a numbered turn: typed by the user and not an interruption.
pub(crate) fn opens_turn(flags: &UserFlags, content: &Content) -> bool {
    is_human_input(flags, content) && !is_interruption(flags, &user_text(content).unwrap_or_default())
}

fn human_text(record: &Record, content: &Content) -> Option<String> {
    if !opens_turn(&record.user_flags(), content) { return None; }
    let text = user_text(content).unwrap_or_default();
    let mut text = clean_prompt(&text);
    if let Content::Blocks(blocks) = content {
        for block in blocks {
            let placeholder = match block.kind.as_deref() {
                Some("image") => "[Image omitted]",
                Some("document") => "[Document omitted]",
                _ => continue,
            };
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(placeholder);
        }
    }
    if text.is_empty() { None } else { Some(text) }
}

/// Read in file order, retaining raw Markdown between XML-style delimiter tags.
/// Bodies and tool summaries are not XML-escaped or wrapped in CDATA.
/// Outer whitespace is trimmed; unreadable controls become U+FFFD.
/// Legacy transcripts often lack stop reasons: the last text-only API message before
/// the next human input / turn-duration marker / EOF is treated as the final answer.
/// Malformed lines are counted, not silently represented as a complete conversation.
pub fn read_transcript(path: &Path) -> Result<TranscriptExport> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let id = path.file_stem().unwrap_or_default().to_string_lossy();
    let mut export = TranscriptExport { text: format!("<transcript session_id=\"{}\">\n\n", xml_attribute(&id)), skipped_lines: 0, tool_hits_hidden: 0 };
    let mut turn = Turn::default();
    let mut filter = RecordFilter::for_file(path);
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let record = match serde_json::from_slice::<Record>(&line) {
            Ok(record) => record,
            Err(_) => {
                export.skipped_lines += 1;
                continue;
            }
        };
        let content = record.content();
        if !filter.keep(record.uuid.as_deref(), record.is_sidechain, record.is_user_input(content.as_ref())) {
            continue;
        }
        if record.kind.as_deref() == Some("system") && record.subtype.as_deref() == Some("turn_duration") {
            turn.finish(&mut export.text);
            continue;
        }
        let (Some(message), Some(content)) = (&record.message, content) else { continue };
        match record.kind.as_deref() {
            Some("user") => {
                if let Some(text) = human_text(&record, &content) {
                    turn.finish(&mut export.text);
                    write_body(&mut export.text, "user", &text);
                }
            }
            Some("assistant") => {
                if turn.completed && (message.id.is_none() || message.id != turn.message_id) {
                    turn.finish(&mut export.text);
                }
                if record.is_aborted {
                    turn.started = true;
                    turn.text.clear();
                    turn.completed = false;
                } else {
                    turn.assistant(message, &content);
                }
            }
            _ => {}
        }
    }
    turn.finish(&mut export.text);
    export.text.push_str("</transcript>\n");
    Ok(export)
}
