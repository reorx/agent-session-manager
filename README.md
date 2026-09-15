# Agent Session Manager

A macOS desktop app (Rust + [GPUI](https://gpui.rs/) via [gpui-kit](https://gpui-kit.com)) that lists every local Claude Code session in a fast, filterable, sortable, searchable table with a detail pane.

Status: M0 done (toolchain + gpui-kit spike, 2026-09-15). The app currently renders 10,000 fake rows; real data lands in M1/M2. See `AGENTS.md` and `kb/plans/2026-09-08-agent-session-manager-prd.md`.

## Requirements

- macOS on Apple Silicon, Xcode 26 with the Metal Toolchain component
- [mise](https://mise.jdx.dev/) for the Rust toolchain (pinned in `mise.toml`)

`make setup` installs both: it runs `mise install`, then downloads the Metal Toolchain if `metal` is missing.

## Development

All common commands are in the `Makefile`. `make` or `make help` lists them.

| Command | What it does |
|---|---|
| `make run` | Build and launch the app (debug) |
| `make run-autoplay` | Same, but scroll / select / sort automatically after launch (`ASM_SPIKE_AUTOPLAY=1`) |
| `make build` / `make release` | Debug build of the workspace / release build of the app |
| `make check` / `make clippy` / `make test` / `make fmt` | The usual cargo tasks across the workspace |
| `make screenshot` | Wake the screen, launch the app and save three window screenshots to `tmp/<date>-screenshots/` (override with `SHOT_DIR=…`) |
| `make bin` | Print the debug binary path |
| `make clean` | Remove only this project's artifacts |

Notes:

- Build output does not go to `./target`. The global `~/.cargo/config.toml` points every project at `~/Library/Caches/cargo-target`; `make bin` shows the exact path.
- First build takes about 1.5 minutes (GPUI + Metal shaders); incremental builds take a couple of seconds.
- `make screenshot` needs `wake-screen` on `PATH` (the display must be awake, otherwise captures are black).

## Layout

```text
crates/core   agent-session-core    parsing, SQLite index, search; no GPUI dependency (M1)
crates/app    agent-session-manager gpui-kit UI
scripts/      screenshot.sh, winid.swift (window id lookup for screencapture)
kb/           docs, plans, notes
```
