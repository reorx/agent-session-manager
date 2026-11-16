use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use agent_session_core::claude::paths::ClaudeDir;
use agent_session_core::search::query::Query;
use agent_session_core::transcript::{TranscriptOptions, TurnRange, read_transcript_with, resolve_transcript};
use anyhow::{Context, Result};
use clap::Args;

#[derive(Args)]
pub struct ReaderArgs {
    /// Transcript path, full Claude Code session ID, or unique ID prefix (put it before --grep)
    pub input: String,
    /// Write to a new text file with XML-style tags (.txt) instead of stdout (never overwrite)
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// One line per turn: its number, time and the first line of the prompt
    #[arg(long)]
    pub outline: bool,
    /// Only these turns: N, N-M or N- (turns start at 1)
    #[arg(long, value_name = "RANGE", value_parser = TurnRange::parse)]
    pub turns: Option<TurnRange>,
    /// Only turns where any TERM occurs, and in them the items with a hit and one on each side
    /// (tool calls count only with --tools); `|` separates ways of saying one. Takes every
    /// following word up to the next option; attach a TERM that starts with `-`: --grep=-rf
    #[arg(long, value_name = "TERM", num_args = 1..)]
    pub grep: Vec<String>,
    /// The --grep TERMs are regular expressions
    #[arg(long, requires = "grep")]
    pub regex: bool,
    /// With --grep, also N turns before and after each turn with a hit
    #[arg(long, value_name = "N", requires = "grep")]
    pub context: Option<u32>,
    /// Include the agent's thinking (with --grep, a thought with a hit is shown without it)
    #[arg(long)]
    pub thinking: bool,
    /// Each tool call with its input and output instead of the one-line summary
    #[arg(long)]
    pub tools: bool,
}

/// Invalid arguments found after parsing (exit 2).
#[derive(Debug)]
pub struct UsageError(pub String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

impl ReaderArgs {
    fn options(&self) -> Result<TranscriptOptions> {
        let grep = match self.grep.is_empty() {
            true => None,
            false => Some(Query::parse(&self.grep, self.regex, false).map_err(|e| UsageError(e.to_string()))?),
        };
        Ok(TranscriptOptions {
            outline: self.outline,
            turns: self.turns,
            grep,
            context: self.context.unwrap_or(0),
            thinking: self.thinking,
            tools: self.tools,
        })
    }
}

pub fn run(args: ReaderArgs, claude: &ClaudeDir, db: &std::path::Path, out: &mut impl Write) -> Result<()> {
    let opts = args.options()?;
    for i in opts.grep.iter().flat_map(Query::regex_fallback) {
        eprintln!("term {} is not a valid regular expression, matched as literal text", i + 1);
    }
    let favorites = ClaudeDir::new(agent_session_core::config::favorites_dir(db)?.join("claude"));
    let path = resolve_transcript(claude, Some(&favorites), &args.input)?;
    let export = read_transcript_with(&path, &opts)?;
    if export.skipped_lines > 0 {
        eprintln!("warning: skipped {} malformed/incomplete JSONL records in {}", export.skipped_lines, path.display());
    }
    if export.tool_hits_hidden > 0 {
        let n = export.tool_hits_hidden;
        let (turns, have) = if n == 1 { ("turn", "has") } else { ("turns", "have") };
        eprintln!("note: {n} {turns} {have} hits only in tool calls; add --tools to read them");
    }
    if let Some(output) = args.output {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&output)
            .with_context(|| format!("create {} (output must not already exist)", output.display()))?;
        file.write_all(export.text.as_bytes())?;
        file.flush()?;
    } else {
        out.write_all(export.text.as_bytes())?;
        out.flush()?;
    }
    Ok(())
}
