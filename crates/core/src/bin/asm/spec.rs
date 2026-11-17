//! The CLI matches its specification, `skills/asm/references/cli.md`: every command clap knows
//! has a `### asm <name>` section there, and every option and argument a row in its table. The direction is one way on purpose: the spec is
//! written before the code, so it may describe what does not exist yet, but never lag behind.

use std::path::Path;

use clap::{Command, CommandFactory};

use crate::Cli;

const SPEC: &str = "../../skills/asm/references/cli.md";

fn spec() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(SPEC);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The text under the heading `heading` (exact line), up to the next heading of the same or a
/// higher level. Lines inside code fences are never headings (`# comment` in an example).
fn section(spec: &str, heading: &str) -> Option<String> {
    let level = heading.bytes().take_while(|b| *b == b'#').count();
    let mut body: Option<String> = None;
    let mut in_fence = false;
    for line in spec.lines() {
        if line.starts_with("```") {
            in_fence = !in_fence;
        }
        let hashes = if in_fence { 0 } else { line.bytes().take_while(|b| *b == b'#').count() };
        let is_heading = hashes > 0 && line.as_bytes().get(hashes) == Some(&b' ');
        match &mut body {
            None if is_heading && line.trim_end() == heading => body = Some(String::new()),
            None => {}
            Some(_) if is_heading && hashes <= level => break,
            Some(text) => {
                text.push_str(line);
                text.push('\n');
            }
        }
    }
    body
}

/// `token` occurs in `text` as a whole: `--in` is not found in `--include-self`, nor `-n` in
/// `--no-scan`, nor `ID` in `IDS`.
fn mentions(text: &str, token: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    text.match_indices(token).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + token.len()..].chars().next();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

/// `token` names a row of an options table in `section`: it is in the row's first cell. A
/// mention in a usage line or in prose does not count, every option needs its own row.
fn described(section: &str, token: &str) -> bool {
    section.lines().filter_map(|line| line.trim_start().strip_prefix('|')).any(|row| mentions(row.split('|').next().unwrap_or(""), token))
}

/// How each argument of `cmd` has to appear in its section: `--long`, `-s`, and the value name
/// of a positional argument (`ID`, `TERM`).
fn tokens(cmd: &Command) -> Vec<String> {
    let mut out = Vec::new();
    for arg in cmd.get_arguments() {
        if let Some(long) = arg.get_long() {
            out.push(format!("--{long}"));
        }
        if let Some(short) = arg.get_short() {
            out.push(format!("-{short}"));
        }
        if arg.is_positional() {
            let name = arg.get_value_names().and_then(|names| names.first()).map(|n| n.to_string());
            out.push(name.unwrap_or_else(|| arg.get_id().as_str().to_uppercase()));
        }
    }
    out
}

fn check_commands(spec: &str, cmd: &Command, path: &str, missing: &mut Vec<String>) {
    for sub in cmd.get_subcommands() {
        let name = format!("{path} {}", sub.get_name());
        let heading = format!("### {name}");
        match section(spec, &heading) {
            None => missing.push(format!("`{name}`: no `{heading}` section")),
            Some(body) => {
                for token in tokens(sub) {
                    if !described(&body, &token) {
                        missing.push(format!("`{name}`: `{token}` has no row in its section"));
                    }
                }
            }
        }
        check_commands(spec, sub, &name, missing);
    }
}

/// Everything the spec lacks, one line each.
fn missing_from(spec: &str, cli: &Command) -> Vec<String> {
    let mut missing = Vec::new();
    match section(spec, "### Global options") {
        None => missing.push("no `### Global options` section".into()),
        Some(body) => {
            for token in tokens(cli) {
                if !described(&body, &token) {
                    missing.push(format!("global option `{token}` has no row under Global options"));
                }
            }
        }
    }
    check_commands(spec, cli, "asm", &mut missing);
    missing
}

#[test]
fn every_command_option_and_argument_is_in_the_spec() {
    let missing = missing_from(&spec(), &Cli::command());
    assert!(missing.is_empty(), "{SPEC} is behind the CLI:\n{}", missing.join("\n"));
}

#[test]
fn the_check_wants_a_section_per_command_and_a_row_per_option() {
    let cli = Command::new("asm")
        .arg(clap::Arg::new("db").long("db"))
        .subcommand(
            Command::new("show")
                .arg(clap::Arg::new("id").value_name("ID"))
                .arg(clap::Arg::new("in").long("in"))
                .arg(clap::Arg::new("include_self").long("include-self"))
                .arg(clap::Arg::new("limit").short('n').long("limit")),
        )
        .subcommand(Command::new("gone"));
    let spec = "\
### Global options
| `--db FILE` | file |
### asm show
```
# a comment in an example is not a heading
asm show ID [--in KINDS] [-n N]
```
| Option | Meaning |
|---|---|
| `ID` | the session |
| `--include-self` | with `--in` and `-n, --limit` named only in prose |
### asm other
| `--in` | elsewhere |
| `-n, --limit N` | elsewhere |
";
    assert_eq!(missing_from(spec, &cli), [
        "`asm show`: `--in` has no row in its section",
        "`asm show`: `--limit` has no row in its section",
        "`asm show`: `-n` has no row in its section",
        "`asm gone`: no `### asm gone` section",
    ]);
}

#[test]
fn cache_calls_requires_its_own_spec_row() {
    let text = spec();
    let without = text.lines().filter(|line| !line.starts_with("| `--calls`")).collect::<Vec<_>>().join("\n");
    assert!(missing_from(&without, &Cli::command()).contains(&"`asm cache`: `--calls` has no row in its section".to_string()));
}
