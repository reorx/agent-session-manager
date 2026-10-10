//! Standalone Transcript Reader; shares its implementation with `asm transcript`.
use std::path::PathBuf;
use std::process::ExitCode;
use agent_session_core::claude::paths::ClaudeDir;
use clap::Parser;

#[path = "support/transcript.rs"]
mod transcript;

#[derive(Parser)]
#[command(name = "transcript-reader", version, about = "Export a Claude Code transcript as compact text with XML-style tags for agent handoffs")]
struct Cli {
    /// Claude data directory (default: $CLAUDE_CONFIG_DIR or ~/.claude)
    #[arg(long, env = "ASM_CLAUDE_DIR")]
    claude_dir: Option<PathBuf>,
    #[command(flatten)]
    args: transcript::ReaderArgs,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let claude = cli.claude_dir.map(ClaudeDir::new).unwrap_or_else(ClaudeDir::from_env);
    match transcript::run(cli.args, &claude, &mut std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}
