# Agent Session Manager

A macOS desktop app (Rust + [GPUI](https://gpui.rs/) via [gpui-kit](https://gpui-kit.com)) that lists every local Claude Code session in a fast, filterable, sortable, searchable table with a detail pane.

It reads your Claude Code data in `~/.claude` and keeps its own index; it never changes that data.

## Install

Requires macOS 14 or later on Apple Silicon.

```sh
brew install --cask reorx/tap/agent-session-manager
```

If Homebrew says it cannot load the cask, run `brew trust --tap reorx/tap` first. Or download `AgentSessionManager-<version>.dmg` from [Releases](https://github.com/reorx/agent-session-manager/releases) and drag the app into Applications.

The app checks for updates once a day and asks before installing; you can also choose **Agent Session Manager → Check for Updates…**. Its index, view state and favorites live in `~/Library/Application Support/dev.reorx.AgentSessionManager/`.

## `asm` CLI and agent skill

`asm` reads the same data from the command line, in a form meant for agents: find sessions by
what was said in them, show one session's details, list projects, read a session a few turns
at a time, and check how well the prompt cache was reused. It is not installed with the app
yet; build it from a checkout of this repository (see Requirements):

```sh
make install-cli    # release build, copied to ~/.local/bin/asm; run it again after pulling updates
```

```sh
asm search '连不上|无法连接|offline' 'healthz|健康检查' -d 30   # sessions that contain every TERM; | separates ways of saying one
asm search 'ECONNRESET' --in tool-output -d 7                  # also search tool calls and their output
asm session 3c7e91a2                    # an ID or unique prefix from search: title, project, cost, files, resume command
asm projects -d 30                      # projects active in the last 30 days
asm transcript 3c7e91a2 --outline       # one line per turn
asm transcript 3c7e91a2 --turns 4 --grep healthz   # only the parts of turn 4 around the hits
asm cache                               # last 7 days: cache hit rate, cache rebuilds by cause and their cost, sessions to look at
asm cache 3c7e91a2 --calls              # one session: each rebuild with its turn, cause and cost, then every call
```

- The first command builds the CLI's own index (`index-cli.sqlite` in the app's data directory,
  under a minute); after that each command catches up in well under a second. It keeps its own
  index while the app's is on an older layout, and shares the app's favorites.
- `search`, `session`, `projects` and `cache` take `--json`. Output is colored only in a terminal.
- The session you run `asm` from is left out of `search`, `projects` and the `cache` overview
  unless you add `--include-self`.
- `ASM_CLAUDE_DIR` / `ASM_DB` override the data directory and the index. `asm --help` lists the
  other commands (`list`, `cost`, `stats`, `index`, favorites), and
  [`skills/asm/references/cli.md`](skills/asm/references/cli.md) describes every option.

The **`asm` skill** in [`skills/asm/`](skills/asm/SKILL.md) teaches Claude Code to use the CLI:
ask "帮我找一下近期关于某个 app 连不上网的问题解决会话" or "where did we fix the flaky ECONNRESET?" and it
searches, reads the matching turns, and answers with the session and the command to resume it;
ask "看看最近 7 天缓存命中得好不好" and it reports the hit rate, why and where the cache was rebuilt,
what that cost, and what could be done differently, citing Anthropic's documentation.
Claude Code sessions started in this repository load it automatically.

## Transcript Reader

Export a local Claude Code session as compact text (`.txt`) with XML-style delimiters for another agent:

```sh
make transcript-reader ARGS="SESSION_ID -o session.txt"
make asm ARGS="transcript SESSION_ID" > session.txt
# Build a portable binary in this project (macOS Apple Silicon):
make build-transcript-reader
./dist/transcript-reader SESSION_ID -o session.txt
# Or install the standalone command:
cargo install --path crates/core --bin transcript-reader
transcript-reader 3c7e91a2 -o session.txt
transcript-reader /path/to/transcript.jsonl > session.txt
```

Accepts full session IDs, unique prefixes, or JSONL paths. `--claude-dir` / `ASM_CLAUDE_DIR`
override the data directory (otherwise `CLAUDE_CONFIG_DIR`, then `~/.claude`). No SQLite
index, network access, or model is needed. `-o` refuses to overwrite existing files.

The export preserves cleaned user prompts and final assistant Markdown inside
`<user>` / `<agent>` delimiters, with plain-text `<turn-summary>` entries immediately
before the corresponding answer. Thinking, tool arguments/results, injected context,
compact summaries and intermediate commentary are omitted. Images/documents become short
placeholders. Missing final answers are marked; malformed records produce a stderr warning.
Explicitly completed answers are kept even when an automatic continuation follows.
For older records without stop reasons, the last text-only API message before the next
human prompt, turn-end marker or EOF is used. Consecutive blocks with the same message ID
are kept together. This is deterministic filtering, not an AI-generated summary or a token cap.

The document has a `<transcript session_id="…">` root. Each opening and closing tag is
on its own line, with the body or summary between them. Summaries contain a sentence
such as `Called Bash 2 times, Read 1 time.`, with tools sorted by name and no nested
elements. No tool calls means no summary element.
An agent turn without a final answer has `status="missing"`. These tags separate content
for agents to read; the export is not a strict XML document. Bodies and summaries are raw
text, with no CDATA wrapper or entity escaping: Markdown, literal tags, `&` and `]]>`
remain as written. The session ID attribute is escaped to preserve its quoted boundary.
Unreadable controls are replaced with U+FFFD. Output is UTF-8; use `.txt` (the CLI does not enforce an extension).
This replaces the previous Markdown export; there is no legacy format switch.

IDs resolve main transcripts; pass a `subagents/*.jsonl` path to export a subagent.
History-only sessions cannot be exported. Records stay in file order (no conversation-tree
branch selection). The reusable `transcript::read_transcript` API lives in the core crate;
a GUI export action can call it on a background thread.

## Requirements

- macOS on Apple Silicon, Xcode 26 with the Metal Toolchain component
- [mise](https://mise.jdx.dev/) for the Rust toolchain (pinned in `mise.toml`)

`make setup` installs both: it runs `mise install`, then downloads the Metal Toolchain if `metal` is missing.

## Development

All common commands are in the `Makefile`. `make` or `make help` lists them.

| Command | What it does |
|---|---|
| `make dev` (or `make dv`) | Build and launch the app: the usual dev loop |
| `make dev-autoplay` | Same, but scroll / select / sort automatically after launch (`ASM_SPIKE_AUTOPLAY=1`) |
| `make run` / `make run-autoplay` | Launch the existing binary without rebuilding |
| `make build` / `make release` | Debug build of the workspace / release build of the app |
| `make check` / `make clippy` / `make test` / `make fmt` | The usual cargo tasks across the workspace |
| `make asm ARGS="list -d 7"` | Run the `asm` CLI (release build) |
| `make install-cli` | Copy a release `asm` to `~/.local/bin/asm` (`PREFIX=…` to change); a copy, so run it again after updating |
| `make parity` | Diff `asm` against `~/Code/scripts/claude_session_search.py` on the real `~/.claude` |
| `make screenshot` | Wake the screen, launch the app and save three window screenshots to `tmp/<date>-screenshots/` (override with `SHOT_DIR=…`) |
| `make screenshot-search` | Run the scripted search session (`ASM_SEARCH_AUTOPLAY=1`) and save seven screenshots; types and clicks nothing |
| `make screenshot-detail` | Tour the Conversation tab (`ASM_DETAIL_AUTOPLAY=1`: turn cards, steps, a search hit in a tool output, special cards, the biggest session, a history-only session, a code block, `]` twice, Metadata) and save up to twelve screenshots; types and clicks nothing |
| `make bin` | Print the debug binary path |
| `make clean` | Remove only this project's artifacts |

Notes:

- Build output does not go to `./target`. The global `~/.cargo/config.toml` points every project at `~/Library/Caches/cargo-target`; `make bin` shows the exact path.
- First build takes about 1.5 minutes (GPUI + Metal shaders); incremental builds take a couple of seconds.
- `make screenshot` needs `wake-screen` on `PATH` (the display must be awake, otherwise captures are black).
- `run` kills any running instance first, so `make dev` always shows the build you just made.

## Layout

```text
crates/core   agent-session-core    parsing, SQLite index, FTS5 / regex search, `asm` CLI; no GPUI dependency
crates/app    agent-session-manager gpui-kit UI
scripts/      screenshot.sh, screenshot-clicks.sh, screenshot-search.sh, screenshot-detail.sh, winid.swift (window id lookup), compare-with-python.sh (parity check)
skills/asm    the asm skill for Claude Code (linked from .claude/skills/asm)
kb/           docs, plans, notes
```
