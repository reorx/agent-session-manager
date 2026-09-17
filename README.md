# Agent Session Manager

A macOS desktop app (Rust + [GPUI](https://gpui.rs/) via [gpui-kit](https://gpui-kit.com)) that lists every local Claude Code session in a fast, filterable, sortable, searchable table with a detail pane.

Status: M1 done (2026-09-17). The core crate parses, indexes (SQLite + FTS5) and searches real sessions, and the `asm` CLI replaces `claude_session_search.py` with byte-identical output; the GUI still renders 10,000 fake rows until M2 wires it to the core. See `AGENTS.md` and `kb/plans/2026-09-08-agent-session-manager-prd.md`.

## `asm` CLI

```sh
cargo run --release -p agent-session-core --bin asm -- index          # build / refresh the index (~2s cold, 0.1s incremental)
cargo run --release -p agent-session-core --bin asm -- list tenderbuddy -d 30
cargo run --release -p agent-session-core --bin asm -- session 2149bb6d
cargo run --release -p agent-session-core --bin asm -- search 'docker compose' -l tool_use:Bash   # regex over transcripts
cargo run --release -p agent-session-core --bin asm -- search --index 重构                          # FTS5 over the index
```

`ASM_CLAUDE_DIR` / `ASM_DB` override the data directory and index location; `NO_COLOR=1` disables colors.

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
scripts/      screenshot.sh, winid.swift (window id lookup), compare-with-python.sh (parity check)
kb/           docs, plans, notes
```
