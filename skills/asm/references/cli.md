# asm command-line reference

This is the specification of the `asm` command line: every command, option, default, output
format, JSON field and exit code. The CLI is built to match this file. A test
(`crates/core/src/bin/asm/spec.rs`) walks the command tree and fails when a command has no
`### asm <name>` section here, or one of its options or arguments has no row of its own in the
section's options table (named in the first cell; global options in the Global options table).
So this file may describe options before they exist, but never lag behind them.

Commands come in two groups:

- **For agents**: `search`, `session`, `projects`, and `transcript` with its reading options.
  They print compact plain text meant to be read as is, and `--json` where noted.
- **For people and parity checks**: `search-old`, `session-old`, `list`, `cost`, `stats`,
  `index`, `favorite`, `unfavorite`, `favorites-dir`. `list`, `search-old` and `session-old`
  print exactly what the reference script `claude_session_search.py` prints (compared byte for
  byte by `scripts/compare-with-python.sh`), so their output never changes.

## Conventions

### Global options

These go before or after the command name and apply to every command.

| Option | Values | Default | Meaning |
|---|---|---|---|
| `--claude-dir DIR` | directory | `$ASM_CLAUDE_DIR`, else `$CLAUDE_CONFIG_DIR`, else `~/.claude` | Claude Code data directory to read |
| `--db FILE` | file | `$ASM_DB`, else `~/Library/Application Support/dev.reorx.AgentSessionManager/index-cli.sqlite` | The SQLite index, see Where the index is |
| `--color WHEN` | `auto`, `always`, `never` | `auto` | ANSI colors. `auto`: only when stdout is a terminal and `NO_COLOR` is unset or empty. `always` wins over `NO_COLOR` |
| `--no-color` | — | — | Same as `--color never`, and wins over `--color` when both are given |
| `--version`, `--help` | — | — | Print the version (`asm 0.1.0`) or the help, then exit 0. The version is the crate's and never changes, so it cannot tell what the CLI can do; see Checking the CLI below |

Output piped to another program or captured by an agent is never colored unless
`--color always` is given.

### Checking the CLI

`asm projects --help` exits 0 only on a binary that has the commands for agents this reference
describes; an older one exits 2 (unrecognized subcommand). Do not check with
`asm search --help`: older binaries have a `search` too, with other options. When this reference
adds commands, it names the one to check here.

### Where the index is

While these commands are developed, `asm` keeps an index of its own, `index-cli.sqlite`, next to
the app's `index.sqlite` in `~/Library/Application Support/dev.reorx.AgentSessionManager/`: the
CLI's index has a newer layout than the released app's, and one file shared by both would be
rebuilt from scratch whenever the other opened it. The two are merged again when the app ships
with these commands (plan P4). `--db` / `ASM_DB` still pick any file, the app's included.
Favorites and their settings live in the same directory and stay shared: a session favorited in
the app is a favorite here too.

### The index and the write lock

A command that reads the index first brings it up to date with the files on disk (an incremental
scan, usually under 0.2 s); `--no-scan` skips that step where the command offers it, except on an
index no scan has gone through yet (see below). `stats` never refreshes. `transcript`, `search-old` without `--index` and `session-old --conversation` read the
record files and do not touch the index at all.

Another `asm` process, or the app when `--db` points at its index, may be writing the same index. When the scan cannot get the write lock within 2 seconds,
the command drops the scan, prints one line on stderr and answers from the index as it is:

```
index busy, results may miss the last few minutes
```

This is not an error: the exit code stays 0. `asm index` is the exception: it exists to write the
index, so it fails (exit 1) when the lock is taken.

Answering from the index as it is needs an index that a scan has gone through. A new index, or
one being rebuilt (`asm index --rebuild`, the app's rebuild, a version change), holds nothing
until its first scan is written, and the process holding the lock on it is writing that scan. So
on such an index the command waits for the lock up to 60 seconds instead, then answers from the
finished index, without the stderr line. A first scan of a large `~/.claude` writes for several
seconds. If the lock is still held after 60 seconds, the command fails (exit 1) and says the
index is being built by another process; run it again later, or point `--db` / `ASM_DB` at
another file. On such an index `--no-scan` is ignored (there is nothing to answer from without
the scan), and `asm index` waits the same way, then counts the files the other process wrote as
`indexed meanwhile by another process`. `asm index --rebuild` does not wait: it fails after 2
seconds, as on any index whose lock is taken.

An index holds the sessions of one Claude directory and one favorites directory, the ones it was
built from. A command run with others fails (exit 1) and suggests `--db` or `asm index --rebuild`,
also when the other directory's build was written while it waited for the lock.

Several `asm` commands may run at once (an agent often starts them in parallel) next to the app:
each refresh writes a file only if no other one indexed it since it looked, so nothing is indexed
twice.

A new index (a new `--db` / `ASM_DB` path) is built by the first command that opens it, and an
index built by another version of `asm` or the app is rebuilt from scratch on first use. Commands
started at the same moment build it once: the others wait for that build and use it. When the
index needs building or rebuilding as the command opens it, and another process keeps it locked
for more than about 2 seconds (an older app writing an index of its own version, say), the command
fails (exit 1), says why the index needs building, and leaves the file alone; wait for that
process, quit the app, or point `--db` / `ASM_DB` at another file. Only a file that is not an
index at all (not an SQLite database, or a damaged one) is deleted and built again. Any other
failure to open the index (a read-only file, a full disk) fails the command with its cause.

### Time range

`search` and `projects` take the same three options:

| Option | Values | Meaning |
|---|---|---|
| `-d, --days N` | number ≥ 0, decimals allowed | Activity within the last N days (N × 24 hours before now). `--days 0` removes the limit |
| `--since DATE` | `YYYY-MM-DD` | Activity on or after local midnight of DATE |
| `--until DATE` | `YYYY-MM-DD` | Activity on or before the end of local day DATE; can be used without `--since` |

`--days` cannot be combined with `--since` or `--until`, and `--since` cannot be later than
`--until` (exit 2 for both). A session is in range when its activity span (first prompt to last
update) overlaps the range: last update at or after the start, first prompt before the end. When
the first prompt time is unknown, the span is the last update alone. A hit can be anywhere in a
session in range, even outside the range. Without any of the three, each command has its own
default, listed with the command.

The first line of the output names the range actually used:

| Range | Label |
|---|---|
| `--days 7` | `last 7 days` (`last 1 day`, `last 0.5 days`) |
| a default of 30 days | `last 30 days (default)` |
| `--since 2026-11-01` | `since 2026-11-01` |
| `--since 2026-11-01 --until 2026-11-14` | `2026-11-01 to 2026-11-14` |
| `--until 2026-11-14` | `until 2026-11-14` |
| none, or `--days 0` | `all time` |

### The current session

When `asm` runs inside a Claude Code session, Claude Code sets `CLAUDE_CODE_SESSION_ID` for it.
`search` and `projects` leave that session out of their results by default, so an agent never
finds the conversation it is having. `--include-self` puts it back. `session ID` and
`transcript ID` always show what they are asked for. When the variable is unset, nothing is
left out.

### Text kinds

A session's searchable text falls into these kinds; `search --in`, the hit counts and the
snippet labels use these names.

| Kind | What it is |
|---|---|
| `prompt` | A user-side record that opens a numbered turn (see Turns): a message the user typed, or a local command or its output (`<command-name>`, `<local-command-stdout>`, `<bash-input>`, `<bash-stdout>`). Not an interruption marker, not a repeat (same uuid) of an earlier user-side record with text or media of its own, not a subagent's (sidechain) record. A pasted image without text is a prompt with empty text |
| `reply` | Text the agent wrote to the user, including commentary between tool calls, not only the final answer |
| `thinking` | The agent's thinking. Only about one block in ten still has its text; the rest kept only a signature |
| `tool-input` | The input of a tool call, as text: a `description` field comes first as `# description` and a `command` field next as `$ command`, whatever the tool (Bash, Agent, Monitor, Write, …); then `key: value` per other field, nested values as JSON with `: ` and `, ` (`file_path: /x/a.rs`, `edits: [{"old_string": "a"}]`). So search an Agent call's description as `# fix the login` or `fix the login`, not `description: …` |
| `tool-output` | The output of a tool call |
| `other` | Text that showed up on the user side without being typed: compact summaries, background task notifications, interruption markers (`[Request interrupted by user]`), and the like. The output of a local command (`<local-command-stdout>`, `<bash-stdout>`) is a `prompt`, since it starts a turn (see Turns) |

`tool` stands for `tool-input,tool-output`, and `all` for every kind. Text Claude Code injects
on the user side for the model (skill bodies, command caveats, image size notes; `isMeta` records)
is never searched.

### Items

Hits are counted per item, and `--same-message` means one item:

- `prompt`, `reply`, `thinking`, `other`: all the text of that kind in one JSONL record. A record
  with both thinking and reply text is two items. Claude Code usually writes each block of a
  response as its own record, so a reply item is usually one block.
- `tool-input`: one tool call (a `tool_use` block); `tool-output`: one tool result (a
  `tool_result` block).

Both ways of searching (index and scan, see `asm search`) count the same items, so `hits`,
`term_hits` and `--same-message` do not depend on which one answers.

### TERMs

`search` and `transcript --grep` take TERMs. Each TERM is one concept; inside it, `|` separates
ways of saying it, and any one of them counts. Quote a TERM so the shell passes it as one word:

```
asm search '连不上|无法连接|offline' 'healthz|健康检查'
```

- Literal text, case-insensitive (Unicode case folding, also for words shorter than 3
  characters, which the full-text index cannot look up by itself).
- Spaces around `|` do not count: `'连不上 | 无法连接'` is `'连不上|无法连接'`, and
  `' foo bar | baz'` is `'foo bar|baz'`. Spaces inside an alternative do.
- Empty alternatives are dropped: `'foo|'` is `foo`. A TERM with nothing left (`''`, `'|'`,
  `' | '`) would match everything and is an error (exit 2).
- At most 64 TERMs (exit 2).
- Literal TERMs cannot contain `|` itself; to search for a vertical bar, use `--regex` and write
  it as `\|`, e.g. `--regex 'a \| b'`.
- A TERM that starts with `-` would be read as an option. In `search`, put `--` before the TERMs
  and every option before it: `asm search --in tool -- '--force' 'push'`. In `transcript --grep`,
  attach it with `=`: `--grep=-rf`. An attached TERM is the only one of its `--grep`; give the
  others to another `--grep`, before or after it: `--grep foo --grep=-rf` (`--grep=-rf foo` is
  an error, exit 2).
- With `--regex`, each TERM is one regular expression (Rust `regex` syntax, case-insensitive
  unless it starts with `(?-i)`), and `|` is the regular expression's own alternation; spaces
  are part of the expression. A TERM that can match the empty string, an empty match at a
  position included, is an error (exit 2): `'a|'`, `'x*'`, `'\b'`, `'^'`, `'foo|\b'`. A TERM that is not
  a valid regular expression is matched as one literal text, not split at `|`, and stderr says
  `term 2 is not a valid regular expression, matched as literal text` (TERMs count from 1).
- TERMs too large for the regular expression engine are an error (exit 2), not literal text:
  one valid expression too large alone (`--regex '\w{400}'`), or several that are each fine but
  too large together (they are matched as one expression to mark them in snippets), e.g.
  `--regex '\w{200}' 'x\w{200}'`.

### Turns

Turn N is the Nth prompt and everything after it up to the next prompt, automatic exchanges
included. Turn numbers are the same in `asm search` snippets, `asm session`, `asm transcript`
and the Turn numbers of the app's detail panel. Text that comes before the first prompt belongs to
no turn.

A local command takes two turns for now. Claude Code writes a slash command that runs without
the model (`/model`, `/clear`) as a `<command-name>` record followed by a
`<local-command-stdout>` record, and a `!` shell command as `<bash-input>` followed by
`<bash-stdout>` / `<bash-stderr>`. Each of the two records is a prompt and opens a turn, and the
output's turn has no reply.

### Hit marks and snippets

In text output, matched text is wrapped in `«` and `»`, with or without color (color makes it
bold red as well). A snippet is one line: up to 60 characters before the first hit in the item
and 140 from its start, `…` where text was cut, `⏎` for a line break.

### Times

Text output uses local time: `2026-11-14 17:09` for dates, `11-14 17:09` inside snippets. JSON
uses RFC 3339 with the local offset, to the second (`2026-11-14T17:09:55+08:00`), or `null`
when unknown.

### JSON output

With `--json` a command prints one pretty-printed JSON object on stdout and nothing else there.
Keys are `snake_case` (`tool_output` in `hits`) and always present (`null` when there is no value).
String values that name a text kind keep the command-line spelling (`"kind": "tool-output"`). The
same thing has the same key in every command (`main_model`, `project`, `project_path`,
`updated_at`, `turns`). `meta`
repeats the options under the option's own name: `meta.model` is the `--model` value, while a
session's model is its `main_model`. Lists of results look like:

```json
{ "meta": { "command": "search", "elapsed_ms": 31, "...": "..." }, "results": [ ] }
```

`meta.index_refresh` is `"done"` (the index was brought up to date), `"busy"` (the write lock
was taken and the command answered from the index as it was, see above) or `"skipped"`
(`--no-scan` on an index scanned before; on one never scanned it is `"done"`).

When a command fails (exit 1) with `--json`, stdout carries
`{ "error": { "code": "...", "message": "..." } }` and stderr the same message. `code` is
`not_found`, `ambiguous` (with a `candidates` array, see `asm session`) or `failed`.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success, including zero results, and output its reader stopped reading (`asm … \| head`): nothing is printed about the closed pipe |
| 1 | Error: no session matches an ID, an ambiguous ID prefix, an unreadable file, an index error |
| 2 | Invalid arguments (unknown option, bad value, options that cannot go together); the message is on stderr |

## Commands for agents

### asm search

Find sessions by their content. Each TERM is one concept that the session must contain; a TERM
can list several ways of saying it (see TERMs).

```
asm search TERM... [--in KINDS] [--tool NAME]... [--regex] [--same-message]
           [-d N | --since DATE [--until DATE]] [-p P]... [--model M] [--favorites]
           [--include-self] [--sort recent|hits] [-n N] [--snippets N] [--json] [--no-scan]
```

| Option | Values | Default | Meaning |
|---|---|---|---|
| `TERM...` | one or more | required | A concept, written as in TERMs. A session is a result only when every TERM occurs somewhere in it (in different items is fine) |
| `--in KINDS` | comma-separated: `prompt`, `reply`, `thinking`, `tool`, `tool-input`, `tool-output`, `other`, `all` | `prompt,reply,thinking`; `tool` with `--tool` | Which text kinds to search |
| `--tool NAME` | tool name, repeatable | all tools | Only calls of these tools (exact name, case-insensitive, e.g. `Bash`, `mcp__context7__query-docs`). Without `--in`, search only these tools' inputs and outputs (as `--in tool`). With `--in`, narrow the tool kinds it names to these tools and leave its other kinds alone; an `--in` without a tool kind is an error (exit 2) |
| `--regex` | — | off | Each TERM is a regular expression, see TERMs |
| `--same-message` | — | off | All TERMs must occur in one item (see Items) |
| `-d, --days N`, `--since DATE`, `--until DATE` | see Time range | all time when every kind searched is `prompt`, `reply`, `thinking` or `other` and `--regex` is off; else the last 30 days | Sessions active in this range |
| `-p, --project P` | directory or text, repeatable | all projects | Only these projects, see Projects below. Several: any of them. The next word is always P, also one starting with `-` (an encoded directory name) |
| `--model M` | model, e.g. `opus-5-5` or `claude-opus-5-5` | all | Only sessions whose main model (the one that cost the most, as in `asm cost --model`) is M |
| `--favorites` | — | off | Only favorite sessions |
| `--include-self` | — | off | Include the current session (see The current session) |
| `--sort` | `recent`, `hits` | `recent` | `recent`: last activity, newest first. `hits`: most hits first, then newest |
| `-n, --limit N` | number | `20` | At most N sessions; `0` for no limit |
| `--snippets N` | number | `3` | At most N snippets per session; `0` for none |
| `--json` | — | off | JSON output, see below |
| `--no-scan` | — | off | Do not refresh the index first |

How it searches: when every kind searched is in the index (`prompt`, `reply`, `thinking`,
`other`) and `--regex` is off, the full-text index answers (tens of milliseconds). Tool kinds and
`--regex` scan the transcript files of the sessions in range. Every file is read. A file that
cannot contain every TERM is not parsed at all; in the others, the lines parsed are those that
can contain a TERM and the user-side lines with text or media (they number the turns). Reading
dominates, at about half a second per GB of transcripts in range (more when the files are not in
the OS file cache), so give tool searches a time range. Subagent conversations are not searched.

Projects (`--project P`):

- A directory: P is `.` or `..`, or contains `/`, or starts with `~`. A relative path is taken
  from the current directory, `~` is the home directory, `.` and `..` parts are resolved
  without looking at the disk, and a trailing `/` does not matter. The directory need not exist.
  It selects the project at that directory and the projects inside it. Paths are compared both
  as written and with symlinks resolved, on either side, so `/tmp/x` and `/private/tmp/x` are
  the same project. For a directory that no longer exists, the symlinks in the part of its path
  that still exists are resolved (a deleted `/tmp/x` is still `/private/tmp/x`). Only a
  relative path reads the current directory: when it cannot be read (it was deleted since the
  shell went there), the command fails (exit 1) and says so; other values of P, and no P, work
  from such a directory.
- Anything else is text: projects whose path, or encoded directory name under `projects/`,
  contains it (case-insensitive).
- Every value `asm projects` prints works as P as printed, with `-p` too: a path, or the encoded
  directory name (`-Users-me-Code-x`) of a project whose path is unknown.
- When no project matches (no session in the index is in a project P selects, at any time), the
  result is 0 sessions, not an error (unlike `asm list`), and `try:` is only `without --project`:
  widening anything else would find nothing either.

Counting: a hit is one item (see Items) that contains at least one TERM. `hits` counts them per
kind; their sum is what `--sort hits` sorts by.

Snippets: the best items first. An item with more of the TERMs comes first, then by kind
(`prompt`, `reply`, `thinking`, `tool-input`, `tool-output`, `other`), then by position in the
session; the first picks make sure every TERM is shown at least once when the snippet limit
allows.

Text output:

```
2 sessions · terms [连不上|无法连接|offline] [健康检查|healthz] · in prompt,reply,thinking · all time · 0.03s

1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26
  updated 2026-11-14 17:09 · started 2026-11-13 07:40 · cw-edge (main) · 21 turns · $4.10
  title: 现场抓帧任务停止排查
  hits: prompt 1 · reply 4 · thinking 0
  [reply · turn 6 · 11-13 08:06] …第 12 轮开始前健康检查«连不上»现场（`«healthz»_error:URLError`）…
  [prompt · turn 5 · 11-13 08:01] …为什么抓帧停了，«healthz» 报错…
  resume: cd '/Users/reorx/Code/cw-edge' && claude --resume 1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26

9a0c33e1-7f52-4f0e-9d0b-2a1e5c6f8b10
  updated 2026-11-02 21:40 · started 2026-11-02 20:55 · vibench · 7 turns · $1.32
  title: Maiev can't reach the server
  prompt: maiev 一直连不上，帮我看看是不是 healthz 的问题
  hits: prompt 2 · reply 3 · thinking 1
  [prompt · turn 1 · 11-02 20:55] maiev 一直«连不上»，帮我看看是不是 «healthz» 的问题
  [thinking · turn 2 · 11-02 21:03] …the «healthz» endpoint returns 503 while the app is «offline»…
  resume: cd '/Users/reorx/Code/vibench' && claude --resume 9a0c33e1-7f52-4f0e-9d0b-2a1e5c6f8b10
```

- First line: `N sessions`, then `(showing M)` when the limit cut the list, the terms (one `[...]`
  each), ` · regex` and ` · same message` when given, `in` with the kinds (and ` · tools Bash,Read`
  with `--tool`), the filters given (` · project cw-edge`, several joined with `,`;
  ` · model opus-5-5`; ` · favorites`), the range label, and the elapsed time.
- Per session, separated by a blank line: the full session ID; a line with last update, start
  (left out when unknown), project name (last path component) with the git branch in
  parentheses when known, the number of turns, and the cost (left out when unknown); `title:`,
  the session's title or else the first line of its first prompt (one line, at most 120
  characters; `(no prompt)` when there is neither); `prompt:`, the first prompt as one line of at most 160 characters, only when the
  session has a title of its own; `hits:`, every kind searched with its count (zeros included);
  the snippets, each as `[KIND · turn N · MM-DD HH:MM] text`, where a tool kind is followed by
  the tool name (`[tool-output Bash · turn 6 · 11-13 08:06]`), `turn N` is left out for text
  before the first prompt and the time when the item has none; `resume:`, the command that resumes the session, or `resume: none (…)`
  with the reason when it cannot be resumed (see `asm session`).
- When the limit cut the list, the last line is `showing 20 of 57 sessions, use -n for more`.
- With no result the output is the first line (`0 sessions · …`) and a line starting with
  `try: ` that lists, separated by ` · `, what can be widened, in this order and only those that
  apply: `--days 0 for all time` (a time limit was in effect), `--in all for every kind` (not every
  kind was searched), `fewer terms` (more than one TERM), `without --same-message`,
  `without --project` / `--tool` / `--model` / `--favorites` (each one given). When none of
  these applies (all time, every kind, one TERM, no filter), it is `other ways of saying each TERM`.
  When no project matches `--project`, it is only `without --project` (see Projects).

JSON output (`--json`):

```json
{
  "meta": {
    "command": "search",
    "terms": [["连不上", "无法连接", "offline"], ["健康检查", "healthz"]],
    "regex": false,
    "regex_fallback": [],
    "same_message": false,
    "kinds": ["prompt", "reply", "thinking"],
    "tools": [],
    "range": { "label": "all time", "since": null, "until": null, "default": true },
    "projects": [],
    "model": null,
    "favorites": false,
    "excluded_session": "5d0c2a4e-1111-4c4c-9a9a-0123456789ab",
    "sort": "recent",
    "engine": "index",
    "scan": null,
    "index_refresh": "done",
    "elapsed_ms": 31,
    "total": 2,
    "shown": 2,
    "truncated": false,
    "hints": []
  },
  "results": [
    {
      "session_id": "1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26",
      "title": "现场抓帧任务停止排查",
      "first_prompt": "为什么抓帧停了，healthz 报错",
      "project": "cw-edge",
      "project_path": "/Users/reorx/Code/cw-edge",
      "cwd": "/Users/reorx/Code/cw-edge",
      "branch": "main",
      "started_at": "2026-11-13T07:40:12+08:00",
      "updated_at": "2026-11-14T17:09:55+08:00",
      "turns": 21,
      "cost_usd": 4.1,
      "main_model": "claude-opus-5-5",
      "kind": "cli",
      "favorite": false,
      "transcript": "/Users/reorx/.claude/projects/-Users-reorx-Code-cw-edge/1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26.jsonl",
      "resume": "cd '/Users/reorx/Code/cw-edge' && claude --resume 1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26",
      "hits": { "total": 5, "prompt": 1, "reply": 4, "thinking": 0, "tool_input": null, "tool_output": null, "other": null },
      "term_hits": [4, 2],
      "snippets": [
        {
          "kind": "reply",
          "tool": null,
          "turn": 6,
          "time": "2026-11-13T08:06:31+08:00",
          "offset": 183402,
          "terms": [0, 1],
          "text": "…第 12 轮开始前健康检查«连不上»现场（`«healthz»_error:URLError`）…"
        }
      ]
    }
  ]
}
```

| Field | Meaning |
|---|---|
| `meta.terms` | One array per TERM, its alternatives as given, empty ones dropped (the whole TERM is one entry with `--regex`) |
| `meta.regex_fallback` | With `--regex`, the indexes (from 0, into `meta.terms`) of the TERMs that did not compile and were matched as literal text |
| `meta.kinds` | Kinds searched, `tool` and `all` expanded, in the order of the Text kinds table, spelled as on the command line (`tool-input`) |
| `meta.tools` | `--tool` values |
| `meta.range` | `label` as in the first line; `since` and `until` the instants it covers (`until` exclusive), `null` when open; `default` is true when no range option was given |
| `meta.projects`, `meta.model`, `meta.favorites` | The filters as given |
| `meta.excluded_session` | The current session's ID when it is left out (`CLAUDE_CODE_SESSION_ID` set, no `--include-self`), whether or not it would have matched; else `null` |
| `meta.engine` | `index` or `scan` (see How it searches) |
| `meta.scan` | With the `scan` engine, what it read: `files` (transcripts in range), `skipped_files` (skipped whole because a TERM is missing from their bytes), `bytes`, `skipped_bytes`; `null` with the index |
| `meta.total`, `meta.shown`, `meta.truncated` | Sessions found, sessions in `results`, whether the limit cut the list |
| `meta.hints` | The `try:` suggestions of an empty result, each as a string; empty otherwise |
| `title` | The session's title, else the first line of its first prompt, cut as in the text output (one line, at most 120 characters); `null` when there is neither |
| `first_prompt` | The first prompt as `asm session` defines it (the first user line with text, cleaned of injected tags), at most 2,000 characters (`…` when cut); `null` when there is none |
| `project` | Last component of `project_path` (else of `cwd`, else the encoded directory name under `projects/`) |
| `project_path`, `cwd` | The project directory and the working directory the session started in |
| `turns` | Number of turns, counted where the snippets' turn numbers come from (the index, or the transcript as the scan read it), so no snippet's turn is above it |
| `cost_usd` | Cost priced from token usage, subagents included; `null` when no call could be priced |
| `main_model` | Main model (the one that cost the most), `null` without API calls |
| `kind` | `cli`, `sdk`, `background` or `history` (the app's Kind column) |
| `transcript` | Path of the record file that was searched |
| `resume` | The resume command, `null` when the session cannot be resumed (see `asm session`) |
| `hits` | `total`, then every kind with its count: `prompt`, `reply`, `thinking`, `tool_input`, `tool_output`, `other`; `null` for a kind not searched |
| `term_hits` | Per TERM, how many items contain it |
| `snippets[].kind`, `.tool` | Text kind; the tool name for tool kinds, else `null` |
| `snippets[].turn`, `.time` | Turn number (`null` before the first prompt) and the item's time |
| `snippets[].offset` | Byte offset, in `transcript`, of the JSONL line the item comes from |
| `snippets[].terms` | Indexes (into `meta.terms`) of the TERMs the item contains |
| `snippets[].text` | The snippet, with `«»` marks |

Exit codes: 0, also for zero results; 1 on an index error; 2 for invalid arguments: an unknown
kind in `--in`, a TERM with nothing to match (see TERMs), `--tool` with an `--in` that names no
tool kind, `--days` with `--since`, `--since` later than `--until`.

### asm session

Show one session's metadata: what it was about, where it ran, how big it got, and how to resume
it.

```
asm session ID [--json] [--no-scan]
```

| Option | Values | Default | Meaning |
|---|---|---|---|
| `ID` | full session ID or a unique prefix | required | The session. An empty or blank ID is an invalid argument (exit 2, `error: ID is empty`), not a prefix of every session |
| `--json` | — | off | JSON output |
| `--no-scan` | — | off | Do not refresh the index first |

Text output:

```
1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26
  title:        现场抓帧任务停止排查
  first prompt: 为什么抓帧停了，healthz 报错
  last prompt:  好的，提交吧
  project:      /Users/reorx/Code/cw-edge
  cwd:          /Users/reorx/Code/cw-edge
  branch:       main
  kind:         cli
  favorite:     no
  started:      2026-11-13 07:40
  updated:      2026-11-14 17:09
  turns:        21 (23 prompts)
  tools:        Bash 120 · Read 45 · Edit 30 · Grep 12 · Write 3 · Glob 2 · Task 1 · TodoWrite 1 (+2 more)
  files:        5 changed: src/edge/health.py, src/edge/capture.py, tests/test_health.py, README.md, Makefile
  cost:         $4.10 (Claude Code recorded $4.02)
  tokens:       in 1.2M · out 50k · cache read 10M · cache write 500k · hit 87.3%
  context:      412k / 1M (41%) at the end · peak 655k
  models:       claude-opus-5-5 (main), claude-haiku-4-5
  subagents:    3
  transcript:   /Users/reorx/.claude/projects/-Users-reorx-Code-cw-edge/1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26.jsonl (2.1 MB)
  resume:       cd '/Users/reorx/Code/cw-edge' && claude --resume 1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26
```

- Prompts are one line of at most 300 characters. A missing value is `—`.
- `first prompt`: the first user line with text, cleaned of injected tags, as the app's table
  and `asm list` show it. `last prompt`: the last prompt (see Text kinds) with text, cleaned of
  injected tags, so never a task notification, a compact summary or an interruption marker; for a
  history-only session, the last prompt in `history.jsonl`. The app's Last prompt can differ:
  it takes the last user line with text, the reference script's way.
- `turns`: turns as defined above, then in parentheses the Prompts count of the app's table and
  of `asm list`, which counts user-side lines the reference script's way (task notifications and
  compact summaries included, images pasted without text not), so the two differ either way.
- `tools`: calls per tool, most used first, at most 8 named. `files`: files the session wrote or
  edited, relative to the project when inside it, at most 10 named, then `(+N more)`.
- `cost`: priced from token usage, subagents included; the cost Claude Code recorded itself in
  parentheses when it recorded one.
- `tokens`: `in` is uncached input; `hit` is cache reads over all input (uncached, cache writes and
  cache reads), to one decimal, rounded down, as in the app; left out without input.
- `context`: the last main-thread prompt against the model's context window, then the peak.
- `transcript`: the record file and its size; `(favorite copy)` after it when only the copy kept
  by favoriting is left; `none (history only)` when Claude Code kept only the prompts in
  `history.jsonl`, so there is nothing to read or resume.
- `resume`: the command to continue the session in Claude Code: `cd 'PROJECT' && claude --resume ID`
  when the project directory exists, else `claude --resume ID`. `none (history only)` or
  `none (only a favorite copy is left)` when Claude Code no longer has the transcript, and
  `none (no plain session id)` for an ID that is not letters, digits and `-` (never put on a
  command line).

When the prefix matches several sessions, nothing goes to stdout; stderr lists them (ID, last
update, title as in `asm search`), most recently updated first, at most 20 and then a line
`  … N more`, and the exit code is 1:

```
prefix '1b2b' matches 3 sessions:
  1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26  2026-11-14 17:09  现场抓帧任务停止排查
  1b2b0f17-2a9e-4c1d-8f3e-7d6b5a4c3b2a  2026-10-30 11:02  Fix the release script
  1b2b9c44-0d1e-4f2a-b3c4-d5e6f7a8b9c0  2026-09-12 08:45  (no prompt)
```

With no match: `no session matches '1b2b'` on stderr, exit 1.

JSON output:

```json
{
  "meta": { "command": "session", "query": "1b2b", "index_refresh": "done", "elapsed_ms": 4 },
  "session": {
    "session_id": "1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26",
    "title": "现场抓帧任务停止排查",
    "first_prompt": "为什么抓帧停了，healthz 报错",
    "last_prompt": "好的，提交吧",
    "project": "cw-edge",
    "project_path": "/Users/reorx/Code/cw-edge",
    "cwd": "/Users/reorx/Code/cw-edge",
    "branch": "main",
    "kind": "cli",
    "favorite": false,
    "started_at": "2026-11-13T07:40:12+08:00",
    "updated_at": "2026-11-14T17:09:55+08:00",
    "turns": 21,
    "prompts": 23,
    "tools": { "Bash": 120, "Read": 45 },
    "files": ["src/edge/health.py", "src/edge/capture.py"],
    "cost_usd": 4.1,
    "recorded_cost_usd": 4.02,
    "tokens": { "input": 1200000, "output": 50000, "cache_read": 10000000, "cache_write": 500000, "hit_rate": 0.873 },
    "context": { "last": 412000, "peak": 655000, "window": 1000000 },
    "main_model": "claude-opus-5-5",
    "models": ["claude-haiku-4-5", "claude-opus-5-5"],
    "subagents": 3,
    "transcript": "/Users/reorx/.claude/projects/-Users-reorx-Code-cw-edge/1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26.jsonl",
    "transcript_bytes": 2202009,
    "favorite_copy": false,
    "resume": "cd '/Users/reorx/Code/cw-edge' && claude --resume 1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26"
  }
}
```

| Field | Meaning |
|---|---|
| `first_prompt`, `last_prompt` | As in the text lines, at most 2,000 characters (`…` when cut); `null` when there is none |
| `turns`, `prompts` | As in the `turns` line |
| `tools` | Every tool with its call count |
| `files` | Every file written or edited, as in the `files` line but not cut |
| `recorded_cost_usd` | What Claude Code recorded itself, `null` when nothing |
| `tokens` | Token sums over every API response, subagents included; `input` is uncached input; `hit_rate` is cache reads over all input as an exact ratio (0–1, `null` without input) |
| `context` | `last` and `peak` main-thread prompt tokens, `window` of the model; `null` each when unknown |
| `transcript`, `transcript_bytes` | `null` for a history-only session |
| `favorite_copy` | True when `transcript` is the copy kept by favoriting because the original is gone |
| `resume` | `null` when the session cannot be resumed |

The other fields are as in `asm search`. With an ambiguous prefix, stderr has only the first line
(`prefix '1b2b' matches 3 sessions:`), and stdout carries
`{ "error": { "code": "ambiguous", "message": "...", "candidates": [{ "session_id": "...", "updated_at": "...", "title": "..." }] } }`
(all candidates, not only 20); with no match, `code` is `not_found`.

Exit codes: 0; 1 when no session or several sessions match; 2 for invalid arguments, an empty ID
included.

### asm projects

List the projects that have sessions, most recently active first. Use it to find the `--project`
value when the user names an app or a repository loosely.

```
asm projects [-d N | --since DATE [--until DATE]] [-n N] [--include-self] [--json] [--no-scan]
```

| Option | Values | Default | Meaning |
|---|---|---|---|
| `-d, --days N`, `--since DATE`, `--until DATE` | see Time range | all time | Only projects with a session active in the range |
| `-n, --limit N` | number | `50` | At most N projects; `0` for no limit |
| `--include-self` | — | off | Count the current session too |
| `--json` | — | off | JSON output |
| `--no-scan` | — | off | Do not refresh the index first |

Text output:

```
3 projects · last 30 days · 0.01s
2026-11-14 17:09   21 sessions  /Users/reorx/Code/cw-edge
2026-11-13 10:02    3 sessions  /Users/reorx/Code/vibench
2026-11-02 09:15    1 session   /Users/reorx/Code/scripts
```

- First line: `N projects`, `(showing M)` when cut, the range label, the elapsed time.
- One line per project: last activity, the number of sessions active in the range (all of the
  project's sessions without a range, history-only ones included), and the project path (the
  encoded directory name, starting with `-`, when the path is unknown). Either works as a
  `--project` value as printed, `-p -Users-me-Code-x` included.
- Ties in last activity go by path. When the limit cut the list, the last line is
  `showing 50 of 216 projects, use -n for more`.
- The current session is left out of the counts and of the last activity unless
  `--include-self`; a project whose only session in range is the current one is not listed.

JSON output:

```json
{
  "meta": { "command": "projects", "range": { "label": "last 30 days", "since": "2026-10-16T17:10:00+08:00", "until": null, "default": false },
            "excluded_session": null, "index_refresh": "done", "elapsed_ms": 9, "total": 3, "shown": 3, "truncated": false },
  "results": [
    { "project_path": "/Users/reorx/Code/cw-edge", "project": "cw-edge", "project_key": "-Users-reorx-Code-cw-edge", "sessions": 21, "updated_at": "2026-11-14T17:09:55+08:00" }
  ]
}
```

| Field | Meaning |
|---|---|
| `project_path` | Project directory, `null` when unknown |
| `project` | Last component of `project_path`, else `project_key` |
| `project_key` | Encoded directory name under `projects/` |
| `sessions` | As in the text line |
| `updated_at` | Last activity: the latest last update of the sessions counted |

Exit codes: 0; 1 on an index error; 2 for invalid arguments.

### asm transcript

Export a session's transcript as compact text with XML-style tags, whole or in part. Without
reading options the output is the full export, unchanged: each turn's prompt, a one-line summary
of the tools called, and the agent's final reply. The reading options let an agent look at the
outline first and then read only the turns it needs.

```
asm transcript INPUT [-o FILE]
asm transcript INPUT [--outline] [--turns RANGE] [--grep TERM... [--regex] [--context N]] [--thinking] [--tools] [-o FILE]
```

| Option | Values | Default | Meaning |
|---|---|---|---|
| `INPUT` | transcript path, full session ID or unique prefix | required | The session. Put it before `--grep`, which takes every following word up to the next option |
| `-o, --output FILE` | file | stdout | Write to FILE instead; FILE must not exist yet |
| `--outline` | — | off | One line per turn instead of the text |
| `--turns RANGE` | `N`, `N-M`, `N-` (turns start at 1) | all | Only these turns (`N-`: from N to the end) |
| `--grep TERM...` | one or more TERMs (see TERMs) | — | Only turns where any TERM occurs, and in them only the items around the hits (see below). Judged on the full text before any cutting, in the prompt, replies and thinking (`asm search`'s default kinds; thinking also without `--thinking`) and `other` text; in tool input and output only with `--tools`, as `asm search` searches them only when `--in` names them. `--grep` can be repeated; a TERM that starts with `-` is attached: `--grep=-rf` |
| `--regex` | — | off | The `--grep` TERMs are regular expressions, see TERMs (needs `--grep`, else exit 2) |
| `--context N` | number | `0` | Also N turns before and after each turn with a hit (needs `--grep`, else exit 2) |
| `--thinking` | — | off | Include the agent's thinking. With `--grep`, a thought with a hit is shown without it |
| `--tools` | — | off | Include each tool call with its input and output instead of the one-line summary |

`--turns` and `--grep` together keep the turns that satisfy both: a turn in the range that has a
hit or is within `--context` of a turn with a hit (that turn may be outside the range).
`--outline` with either lists only the turns they keep. A `--turns` number beyond the last turn
is an error (exit 1, `turn 30 is beyond the last turn (21)`); `0`, `3-2` or anything but `N`,
`N-M`, `N-` is an invalid argument (exit 2). A regular expression TERM that is not valid is
matched as literal text and stderr says so, as in `search`. With `--grep` but not `--tools`, turns
in range left out because their only hits are in tool calls are noted on stderr:
`note: 2 turns have hits only in tool calls; add --tools to read them` (exit 0).

Full export (no reading option):

```
<transcript session_id="1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26">

<user>
为什么抓帧停了，healthz 报错
</user>

<turn-summary>
Called Bash 3 times, Read 2 times.
</turn-summary>
<agent>
健康检查超时是因为……
</agent>

</transcript>
```

Reading options (`--turns`, `--grep`, `--thinking`, `--tools`) change the export so that every
hit `asm search` reports can be found in its turn (a `tool-input` or `tool-output` hit with
`--tools`): each turn shows the items `asm search` searches (see Items) with the text it
searches. So, unlike the full export:

- The opening tag carries the session's number of turns. With `--grep` it adds `matched="N"`,
  the turns kept for a hit of their own, and with `--context` also `context="N"`, the turns kept
  only for being near one: `<transcript session_id="…" turns="21" matched="1" context="2">`.
- Each `<user>` carries its turn number and local time (`ts` is left out when the record has
  none): `<user turn="5" ts="2026-11-13 08:01">`. Its text is the prompt as recorded, with the
  tags Claude Code wraps around it (`<command-name>`, `<system-reminder>`, …), which the full
  export and `--outline` clean away; an image is an `[Image omitted]` line, a document a
  `[Document omitted]` line.
- Every reply the agent wrote is an `<agent>` element in file order, one per JSONL record,
  commentary between tool calls included. When the agent answered but its last words were not a
  final reply (a tool call came after them, or there were none), the turn ends with
  `<agent status="missing">` as in the full export; a turn the agent never answered (a local
  command) has nothing after `<user>`.
- `other` text (compact summaries, task notifications, interruption markers) is an `<other>`
  element where it comes.
- `--thinking` adds a `<thinking>` element per record that has thinking text; blocks with no text
  left are skipped.
- `--tools` adds a `<tool name="Bash">` element per call, holding `<input>` (as in the
  `tool-input` kind) and, once the result came, `<output>` (`<output status="error">` when the
  tool failed); it drops the `<turn-summary>`. A result whose call is not in the same turn is a
  `<tool>` of its own with only `<output>` (`name="?"` when the call is not found at all).
  Without `--tools`, the `<turn-summary>` comes right after `<user>`.
- A record Claude Code wrote twice (same uuid) or a subagent's record inside the main transcript
  is shown like any other, because `asm search` counts it; the full export and the app leave it
  out. None were found in real transcripts (2026-11-16), but where they occur, a turn's tool
  counts can be higher than in the full export.
- An `<input>`, `<output>` or `<other>` body longer than 2,000 characters is cut. Without a
  `--grep` hit inside it, it keeps the first 2,000 characters and ends with a line
  `…[cut N chars]`. With hits inside, it keeps the first 500 characters and, around each hit,
  300 characters before it and 700 from its start; windows that touch or overlap merge (also
  with the first 500), at most 4,000 characters are kept in all, and each gap is a line
  `…[cut N chars]`. A hit whose window would go past 4,000 gets none, nor do the hits after it;
  the last gap counts them: `…[cut 13288 chars, 3 more hits]`, so read them with a narrower
  TERM. Characters are counted as Unicode characters, not bytes. `<user>`, `<agent>` and
  `<thinking>` are never cut. So a hit `asm search` found deep in a long tool output is read
  with `asm transcript ID --turns N --tools --grep` and the same TERMs.
- With `--grep`, a turn kept shows its prompt, each item with a hit, and the one item before and
  after each of those; the prompt counts as the item before the first. Each run of other items
  is one line `…[skipped 14 items, 9120 chars; --turns 6 without --grep shows them]`: the items
  left out and the characters their text shows without `--grep` (bodies cut to 2,000). An item is
  one `<agent>`, `<thinking>`, `<other>` or `<tool>` element; tool calls are items only with
  `--tools`, and thinking without a hit only with `--thinking`. The `<turn-summary>` and
  `<agent status="missing">` are always shown. A turn kept as context has no hit, so it shows its
  prompt and one skipped line. `--turns` without `--grep` shows whole turns.
- A blank line follows `</user>` and the last element of each turn; the elements inside a turn
  follow each other without one.
- Turns not kept are left out without a trace; text before the first prompt is never shown.

```
<transcript session_id="1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26" turns="21">

<user turn="6" ts="2026-11-13 08:06">
那现场为什么连不上？
</user>

<thinking>
The health check runs before round 12…
</thinking>
<agent>
先看一下健康检查的日志。
</agent>
<tool name="Bash">
<input>
# Show the last health checks
$ tail -n 50 logs/health.log
</input>
<output>
2026-11-13 08:05:58 healthz_error: URLError(timed out)
…[cut 5210 chars]
</output>
</tool>
<agent>
第 12 轮开始前健康检查连不上现场……
</agent>

</transcript>
```

The same turn with `--turns 6 --tools --grep healthz`, which hits the tool output only:

```
<transcript session_id="1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26" turns="21" matched="1">

<user turn="6" ts="2026-11-13 08:06">
那现场为什么连不上？
</user>

<agent>
先看一下健康检查的日志。
</agent>
<tool name="Bash">
<input>
# Show the last health checks
$ tail -n 50 logs/health.log
</input>
<output>
2026-11-13 08:05:58 healthz_error: URLError(timed out)
…[cut 5210 chars]
</output>
</tool>
<agent>
第 12 轮开始前健康检查连不上现场……
</agent>

</transcript>
```

Its thinking (no hit, no `--thinking`) is no item. In a turn with 40 tool calls and a hit in the
20th, the output is that call, the item before and after it, and a skipped line on each side.

`--outline`:

```
1b2bda5d-c5e6-4c50-83dd-6e01c9dabf26 · 21 turns · 2026-11-13 07:40 to 2026-11-14 17:09
   1  2026-11-13 07:40  帮我看看抓帧服务的日志
   2  2026-11-13 07:52  继续
   5  2026-11-13 08:01  为什么抓帧停了，healthz 报错
```

The first line names the session, its number of turns (`, 2 matched` added with `--grep`: the
turns with a hit; `, 3 as context` too with `--context`), and its first prompt's and last
record's local times. Then one line per turn kept:
the turn number (right-aligned in 4 columns), the local time (`?` when unknown), and the first
non-empty line of the prompt cleaned of injected tags (`/model default` rather than the
`<command-name>` tags; `[Image omitted]` or `[Document omitted]` for an image or document alone), at most 120 characters, `…` when
cut.

`transcript` reads the record file directly and never touches the index. A transcript with
malformed lines prints `warning: skipped N malformed/incomplete JSONL records in PATH` on stderr
and exports the rest.

Exit codes: 0; 1 when nothing or several sessions match INPUT, the file cannot be read, FILE
exists, or a turn is out of range; 2 for invalid arguments.

## Commands for people and parity checks

These keep the output of `claude_session_search.py` (`list`, `search-old`, `session-old`) or
their own historical format. They print colors in a terminal; their output is not meant to be
parsed and has no `--json`.

### asm search-old

Search message text, one match per line, grouped by transcript; the reference script's search.

```
asm search-old QUERY [-d DAYS] [-p PROJECT] [-r ROLE]... [-l LABEL]... [--case-sensitive] [-C N] [-n N] [--index] [--no-scan]
```

| Option | Values | Default | Meaning |
|---|---|---|---|
| `QUERY` | regular expression | required | Literal text when it does not compile |
| `-d, --days DAYS` | number | `7` | Only transcripts modified in the last DAYS days; `0` for all |
| `-p, --project PROJECT` | text | all | Only projects whose directory name contains it (case-insensitive) |
| `-r, --role ROLE` | `user`, `assistant`, `tool`; repeatable | all | Message roles to search |
| `-l, --label LABEL` | `thinking`, `tool_use:NAME`, `tool_result`; repeatable | all | Only pieces whose label contains it |
| `--case-sensitive` | — | off | Match case |
| `-C, --context N` | number | `80` | Characters of context on each side of a match |
| `-n, --limit N` | number | `200` | Stop after N matches (N sessions with `--index`) |
| `--index` | — | off | Full-text search over the index (user and assistant text only) instead of scanning |
| `--no-scan` | — | off | With `--index`, do not refresh the index first |

Output: per transcript, `PROJECT FILE  (updated …, N matches)`, then each match as
`MM-DD HH:MM [role/label] text`, tool calls on several lines; last line
`N matches in M sessions scanned`. Exit codes: 0; 1 on an error; 2 for invalid arguments.

### asm session-old

Show the sessions whose ID starts with ID, as the reference script's `-s` does; or, with
`--conversation`, a debugging summary of the turn cards the app would build.

```
asm session-old ID [-n N] [--no-scan]
asm session-old ID --conversation [--match PATTERN]
```

| Option | Values | Default | Meaning |
|---|---|---|---|
| `ID` | session ID prefix | required | — |
| `-n, --limit N` | number | `200` | At most N sessions |
| `--no-scan` | — | off | Do not refresh the index first |
| `--conversation` | — | off | Read turn cards straight from the transcript: one `TURN N` / `AUTO …` / `COMPACT` line each, then a `total turns=…` line |
| `--match PATTERN` | regular expression | — | With `--conversation`, count hits per card |

Exit codes: 0; 1 when no session matches; 2 for invalid arguments.

### asm list

List the sessions of a project, newest first; the reference script's `-L`.

```
asm list [PATH] [--favorites] [-d DAYS] [-n N] [--full] [--no-scan]
```

| Option | Values | Default | Meaning |
|---|---|---|---|
| `PATH` | project directory, or text in project directory names | current directory | Several projects can match text |
| `--favorites` | — | off | Only favorites |
| `-d, --days DAYS` | number | all | Only sessions active in the last DAYS days |
| `-n, --limit N` | number | `200` | At most N sessions in all |
| `--full` | — | off | Print first prompts in full instead of one line |
| `--no-scan` | — | off | Do not refresh the index first |

Output: per project, `KEY (N sessions, M with transcript)`, then per session
`STARTED  ID  N prompts, SIZE, updated …` and its first prompt. Exit codes: 0; 1 when no project
matches; 2 for invalid arguments.

### asm cost

What sessions cost (priced from token usage, subagents included) and how full their context got,
most expensive first.

```
asm cost [-p PROJECT] [--since DATE] [--until DATE] [--sort cost|context] [-n N] [--model M]
         [--as MODEL | --compare | --windows] [--no-scan]
```

| Option | Values | Default | Meaning |
|---|---|---|---|
| `-p, --project PROJECT` | directory or text | all | As `asm list`'s PATH |
| `--since DATE`, `--until DATE` | `YYYY-MM-DD` | open | Sessions last updated on or after / on or before these local dates |
| `--sort` | `cost`, `context` | `cost` | Order |
| `-n, --limit N` | number | `30` | Rows shown (totals cover every session) |
| `--model M` | e.g. `opus-5-5` | all | Only sessions whose main model (the one that cost the most) is M |
| `--as MODEL` | e.g. `fable-5-2` | — | Also price each session with MODEL as its main model; other models keep their price |
| `--compare` | — | off | Compare with the cost Claude Code recorded itself (`cost-state`) |
| `--windows` | — | off | Compare catalog and built-in context windows and list sessions whose context share changes |
| `--no-scan` | — | off | Do not refresh the index first |

`--as`, `--compare` and `--windows` exclude each other (exit 2). Exit codes: 0; 1 when no project
matches or MODEL has no price; 2 for invalid arguments.

### asm stats

Print index statistics (path, sessions, history-only sessions, messages, projects, files, size)
without refreshing it. `messages` counts the records with prompt, reply or other text, injected
text included; thinking and images pasted without text are not counted.

```
asm stats
```

Exit codes: 0; 1 when the index cannot be opened.

### asm index

Bring the index up to date, or rebuild it from scratch, and report what was scanned.

```
asm index [--rebuild]
```

| Option | Values | Default | Meaning |
|---|---|---|---|
| `--rebuild` | — | off | Drop everything and index all files again |

Output: one line of scan counts and timings, one line of index totals. Fails (exit 1) when the
app holds the write lock (on an index no scan has gone through yet, it first waits up to 60
seconds for the process building it, unless `--rebuild` is given; see "The index and the write
lock"), or when the Claude directory or favorites directory differs from the one the index was
built from (run `asm index --rebuild`).

### asm favorite

Keep a copy of a session's transcript in the favorites directory and mark it as a favorite.

```
asm favorite ID
```

| Option | Values | Default | Meaning |
|---|---|---|---|
| `ID` | session ID prefix matching one session | required | — |

Prints `Favorited ID`. Exit codes: 0; 1 when ID does not match exactly one session or the session
has no transcript.

### asm unfavorite

Move a session's favorite copy to the trash and unmark it.

```
asm unfavorite ID [--yes]
```

| Option | Values | Default | Meaning |
|---|---|---|---|
| `ID` | session ID prefix matching one session | required | — |
| `--yes` | — | off | Go ahead even when the copy is the last one left (the original is gone) |

Prints `Unfavorited ID`. Exit codes: 0; 1 when ID does not match one session, or the copy is the
last one and `--yes` is missing.

### asm favorites-dir

Print the favorites directory, or move it.

```
asm favorites-dir [PATH | --reset]
```

| Option | Values | Default | Meaning |
|---|---|---|---|
| `PATH` | directory | — | Move the favorites there |
| `--reset` | — | off | Move them back to the default, next to the index |

Prints the directory, and after a move `(moved N, skipped M)`. Exit codes: 0; 1 when
`ASM_FAVORITES_DIR` controls the directory or the move fails; 2 when PATH and `--reset` are both
given.
