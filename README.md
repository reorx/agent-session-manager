# Agent Session Manager

A macOS desktop app (Rust + [GPUI](https://gpui.rs/) via [gpui-kit](https://gpui-kit.com)) that lists every local Claude Code session in a fast, filterable, sortable, searchable table with a detail pane.

Status: M2 done (2026-09-21). The GUI runs on real data: it loads the SQLite index on launch (~200ms to a usable table of 5.5k sessions), then scans incrementally in the background and streams new rows in; a cold index of ~1.3GB takes about 4s with the table filling as it goes. Project / time filters, header sorting, preview modes and the selection persist across launches. The core crate parses, indexes (SQLite + FTS5) and searches sessions, and the `asm` CLI replaces `claude_session_search.py` with byte-identical output. Search in the GUI is next (M3). See `AGENTS.md` and `kb/plans/2026-09-08-agent-session-manager-prd.md`.

## `asm` CLI

```sh
cargo run --release -p agent-session-core --bin asm -- index          # build / refresh the index (~2s cold, 0.1s incremental)
cargo run --release -p agent-session-core --bin asm -- list tenderbuddy -d 30
cargo run --release -p agent-session-core --bin asm -- session 2149bb6d
cargo run --release -p agent-session-core --bin asm -- search 'docker compose' -l tool_use:Bash   # regex over transcripts
cargo run --release -p agent-session-core --bin asm -- search --index 重构                          # FTS5 over the index
```

`ASM_CLAUDE_DIR` / `ASM_DB` override the data directory and index location; `NO_COLOR=1` disables colors.

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
transcript-reader 1cbab92c -o session.txt
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
| `make parity` | Diff `asm` against `~/Code/scripts/claude_session_search.py` on the real `~/.claude` |
| `make screenshot` | Wake the screen, launch the app and save three window screenshots to `tmp/<date>-screenshots/` (override with `SHOT_DIR=…`) |
| `make screenshot-search` | Run the scripted search session (`ASM_SEARCH_AUTOPLAY=1`) and save seven screenshots; types and clicks nothing |
| `make screenshot-detail` | Tour the Conversation tab (`ASM_DETAIL_AUTOPLAY=1`: turn cards, steps, a search hit in a tool output, special cards, the biggest session) and save up to eight screenshots; types and clicks nothing |
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
kb/           docs, plans, notes
```
