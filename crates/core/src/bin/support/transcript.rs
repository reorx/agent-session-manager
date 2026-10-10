use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use agent_session_core::claude::paths::ClaudeDir;
use agent_session_core::transcript::{read_transcript, resolve_transcript};
use anyhow::{Context, Result};
use clap::Args;

#[derive(Args)]
pub struct ReaderArgs {
    /// Transcript path, full Claude Code session ID, or unique ID prefix
    pub input: String,
    /// Write to a new XML-structured text file (.txt) instead of stdout (never overwrite)
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

pub fn run(args: ReaderArgs, claude: &ClaudeDir, out: &mut impl Write) -> Result<()> {
    let path = resolve_transcript(claude, &args.input)?;
    let export = read_transcript(&path)?;
    if export.skipped_lines > 0 {
        eprintln!("warning: skipped {} malformed/incomplete JSONL records in {}", export.skipped_lines, path.display());
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
