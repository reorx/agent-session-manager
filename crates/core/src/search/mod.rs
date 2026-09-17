//! Two search paths: FTS5 over the index (text mode) and a regex scan of the raw transcripts
//! (regex mode, the only way to search tool inputs / outputs).

pub mod fts;
pub mod pyjson;
pub mod regex;

use std::ops::Range;

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
