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
    #[serde(rename = "durationMs")]
    pub(crate) duration_ms: Option<i64>,
    pub(crate) origin: Option<Origin>,
    #[serde(borrow)]
    pub(crate) message: Option<Message<'a>>,
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

pub(crate) fn is_human_input(record: &Record, content: &Content) -> bool {
    if record.is_meta || record.is_compact_summary
        || record.origin.as_ref().and_then(|o| o.kind.as_deref()).is_some_and(|kind| kind != "human") {
        return false;
    }
    if let Content::Blocks(blocks) = content {
        if blocks.iter().any(|b| b.kind.as_deref() == Some("tool_result")) {
            return false;
        }
    }
    let text = user_text(content).unwrap_or_default();
    if text.starts_with("[Image: source:") {
        return false;
    }
    !clean_prompt(&text).is_empty() || matches!(content, Content::Blocks(blocks)
        if blocks.iter().any(|b| matches!(b.kind.as_deref(), Some("image" | "document"))))
}

pub(crate) fn is_interruption(record: &Record, text: &str) -> bool {
    record.interrupted_message_id.is_some() || text.trim().starts_with("[Request interrupted by user")
}

fn human_text(record: &Record, content: &Content) -> Option<String> {
    if !is_human_input(record, content) { return None; }
    let text = user_text(content).unwrap_or_default();
    if is_interruption(record, &text) { return None; }
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
    let mut export = TranscriptExport { text: format!("<transcript session_id=\"{}\">\n\n", xml_attribute(&id)), skipped_lines: 0 };
    let mut turn = Turn::default();
    let is_subagent = path.parent().and_then(Path::file_name).is_some_and(|name| name == "subagents");
    let mut seen = HashSet::new();
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
        if record.is_sidechain && !is_subagent {
            continue;
        }
        if let Some(uuid) = record.uuid.as_ref().filter(|id| !id.is_empty()) {
            if !seen.insert(uuid.clone()) {
                continue;
            }
        }
        if record.kind.as_deref() == Some("system") && record.subtype.as_deref() == Some("turn_duration") {
            turn.finish(&mut export.text);
            continue;
        }
        let Some(message) = &record.message else { continue };
        let Some(raw) = message.content else { continue };
        let content = parse_content(raw);
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
