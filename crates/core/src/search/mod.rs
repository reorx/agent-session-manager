//! Two search paths: FTS5 over the index (text mode) and a regex scan of the raw transcripts
//! (regex mode, the only way to search tool inputs / outputs).

pub mod fts;
pub mod pyjson;
pub mod regex;

use std::ops::Range;

use ::regex::Regex;

/// Text plus the byte ranges to highlight.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Highlighted {
    pub text: String,
    pub ranges: Vec<Range<usize>>,
}

impl Highlighted {
    pub fn plain(text: impl Into<String>) -> Self {
        Highlighted { text: text.into(), ranges: Vec::new() }
    }

    /// Wrap every highlighted range with `open` / `close` (e.g. ANSI codes).
    pub fn render(&self, open: &str, close: &str) -> String {
        let mut out = String::with_capacity(self.text.len() + self.ranges.len() * (open.len() + close.len()));
        let mut pos = 0;
        for r in &self.ranges {
            out.push_str(&self.text[pos..r.start]);
            out.push_str(open);
            out.push_str(&self.text[r.clone()]);
            out.push_str(close);
            pos = r.end;
        }
        out.push_str(&self.text[pos..]);
        out
    }
}

/// What a query matches, shared by the snippets and by whoever highlights hits in full texts.
#[derive(Clone, Debug)]
pub struct Matcher {
    re: Regex,
}

impl Matcher {
    /// Text mode: any of the whitespace-separated words, case-insensitive. `None` for a blank query.
    pub fn words(query: &str) -> Option<Matcher> {
        let mut words: Vec<&str> = query.split_whitespace().collect();
        if words.is_empty() {
            return None;
        }
        // longest first, so a word wins over its own prefix at the same position
        words.sort_by_key(|w| std::cmp::Reverse(w.len()));
        let pattern = words.iter().map(|w| ::regex::escape(w)).collect::<Vec<_>>().join("|");
        let re = regex::compile_query(&pattern, false).expect("escaped words always compile");
        Some(Matcher { re })
    }

    /// Regex mode; a query that is not a valid regex is matched as literal text.
    pub fn regex(query: &str, case_sensitive: bool) -> Matcher {
        Matcher { re: regex::compile_query(query, case_sensitive).expect("escaped query always compiles") }
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.re.is_match(text)
    }

    /// Byte ranges of every non-empty match.
    pub fn ranges(&self, text: &str) -> Vec<Range<usize>> {
        self.re.find_iter(text).map(|m| m.range()).filter(|r| !r.is_empty()).collect()
    }

    /// One line around the first match: `before` characters ahead of it and `after` characters
    /// from its start, every match in that window highlighted. `…` marks cut text, `⏎` a line break.
    pub fn snippet(&self, text: &str, before: usize, after: usize) -> Highlighted {
        let first = self.re.find(text).map(|m| m.start()).unwrap_or(0);
        let start = match before {
            0 => first,
            n => text[..first].char_indices().rev().nth(n - 1).map(|(i, _)| i).unwrap_or(0),
        };
        let end = text[first..].char_indices().nth(after).map(|(i, _)| first + i).unwrap_or(text.len());
        let mut out = Highlighted::default();
        if start > 0 {
            out.text.push('…');
        }
        let mut pos = start;
        for r in self.ranges(&text[start..end]) {
            let (from, to) = (start + r.start, start + r.end);
            out.text.push_str(&text[pos..from].replace('\n', "⏎"));
            let mark = out.text.len();
            out.text.push_str(&text[from..to].replace('\n', "⏎"));
            out.ranges.push(mark..out.text.len());
            pos = to;
        }
        out.text.push_str(&text[pos..end].replace('\n', "⏎"));
        if end < text.len() {
            out.text.push('…');
        }
        out
    }
}
