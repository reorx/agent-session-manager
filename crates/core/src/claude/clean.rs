//! `clean_prompt` from the reference script: strip the wrappers Claude Code injects around what
//! the user typed, and render slash commands as `/name args`.

use std::sync::LazyLock;

use regex::Regex;

static OPENER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(system-reminder|command-message|ide_opened_file|ide_selection)>").unwrap());
static COMMAND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<command-name>(.*?)</command-name>\s*(?:<command-args>(.*?)</command-args>)?").unwrap()
});

/// Strip harness-injected wrappers so the user's own words remain.
pub fn clean_prompt(text: &str) -> String {
    let stripped = strip_tag_blocks(text);
    let out = COMMAND.replace_all(&stripped, |caps: &regex::Captures| {
        let name = caps[1].trim();
        let args = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
        format!("{name} {args}").trim().to_string()
    });
    out.trim().to_string()
}

/// Same as Python's `re.sub(r'<(tag)>.*?</\1>\s*', '', text, flags=re.S)`: the regex crate has no
/// backreferences, so match the closer by hand.
fn strip_tag_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    while let Some(m) = OPENER.find_at(text, pos) {
        let tag = &text[m.start() + 1..m.end() - 1];
        let closer = format!("</{tag}>");
        match text[m.end()..].find(&closer) {
            Some(rel) => {
                out.push_str(&text[pos..m.start()]);
                let mut end = m.end() + rel + closer.len();
                end += text[end..].len() - text[end..].trim_start().len();
                pos = end;
            }
            None => {
                out.push_str(&text[pos..m.end()]);
                pos = m.end();
            }
        }
    }
    out.push_str(&text[pos..]);
    out
}

/// Join a prompt into one line (`⏎` marks line breaks) and cut it after `max_chars` characters.
pub fn one_line(text: &str, max_chars: usize) -> String {
    let one: String = text.replace('\n', "⏎");
    let count = one.chars().count();
    if count > max_chars {
        let mut cut: String = one.chars().take(max_chars).collect();
        cut.push('…');
        cut
    } else {
        one
    }
}
