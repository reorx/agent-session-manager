//! Byte prefilter for the scan: before any JSON is parsed, a transcript must contain every TERM
//! somewhere (else the whole file is skipped), and a line must contain one of them (else it is
//! not parsed).
//!
//! The TERMs are searched in the raw JSONL bytes, case-insensitive for ASCII only. That finds every
//! line whose decoded text has a match, because Claude Code writes text as raw UTF-8 and escapes
//! only `"`, `\` and control characters (checked on every real record, see the P1-3 session
//! note). So each alternative is cut at those characters and at non-ASCII letters with case
//! (whose other case has other bytes), and the longest piece left is searched: wherever the
//! alternative occurs in decoded text, that piece occurs in the raw line. A TERM without such a
//! piece cannot be prefiltered and lets every file and line through.
//!
//! Tool inputs are searched as text built from the JSON (`# description`, `$ command`,
//! `key: value`, nested values as JSON with `: ` and `, `), where the raw line has `"key":"value"`
//! and no spaces. So when they are searched, alternatives are also cut at every `:`, `,`, `$` and
//! `#`, and a piece's leading spaces are dropped.

use regex::bytes::{Regex, RegexBuilder};
use regex_syntax::hir::literal::{ExtractKind, Extractor};

#[cfg(test)]
use super::query::TermMask;
use super::query::{Query, non_ascii_cased};

/// At most this many literals of one regex TERM are searched; more lets the TERM through.
const MAX_REGEX_LITERALS: usize = 64;

/// A byte regex is many times faster here than an Aho-Corasick automaton with ASCII case folding
/// (2026-11-15, 30 days of transcripts: 0.8 s against 7.9 s of thread time).
pub(crate) struct Prefilter {
    /// Per TERM, its pieces; `None` for a TERM that cannot be prefiltered (taken as present
    /// everywhere).
    terms: Vec<Option<Regex>>,
    /// Any piece of any TERM; `None` when some TERM cannot be prefiltered (every line passes).
    any: Option<Regex>,
}

impl Prefilter {
    pub(crate) fn new(query: &Query, tool_input: bool) -> Prefilter {
        let mut terms = Vec::new();
        let mut all: Vec<String> = Vec::new();
        let mut open = false;
        for term in query.terms() {
            let needles = if query.regex && !term.fallback {
                regex_needles(&term.alternatives[0], tool_input)
            } else {
                term.alternatives.iter().map(|a| needle(a, tool_input)).collect::<Option<Vec<_>>>()
            };
            match needles.as_deref().and_then(any_of) {
                Some(re) => {
                    terms.push(Some(re));
                    all.extend(needles.expect("built from them"));
                }
                None => {
                    open = true;
                    terms.push(None);
                }
            }
        }
        let any = if open { None } else { any_of(&all) };
        Prefilter { terms, any }
    }

    /// Every TERM occurs somewhere in the file (or cannot be checked).
    pub(crate) fn file_may_match(&self, bytes: &[u8]) -> bool {
        self.terms.iter().flatten().all(|re| re.is_match(bytes))
    }

    /// Some TERM may occur in the line.
    pub(crate) fn line_may_match(&self, line: &[u8]) -> bool {
        self.any.as_ref().is_none_or(|re| re.is_match(line))
    }

    #[cfg(test)]
    fn open(&self) -> TermMask {
        self.terms.iter().enumerate().filter(|(_, t)| t.is_none()).fold(0, |m, (i, _)| m | 1 << i)
    }

    #[cfg(test)]
    fn pieces_of(&self, term: usize) -> usize {
        self.terms[term].as_ref().map_or(0, |re| re.as_str().split('|').count())
    }
}

/// Any of the pieces, ASCII case folded, everything else byte for byte; `None` when too many to
/// compile (then nothing is ruled out).
fn any_of(needles: &[String]) -> Option<Regex> {
    let pattern = needles.iter().map(|n| regex::escape(n)).collect::<Vec<_>>().join("|");
    RegexBuilder::new(&format!("(?i-u:{pattern})")).build().ok()
}

/// What to search in the raw bytes for one literal alternative; `None` when nothing can be.
pub(crate) fn needle(alt: &str, tool_input: bool) -> Option<String> {
    let cut = |c: char| {
        c == '"' || c == '\\' || c.is_control() || c == '\u{fffd}' || non_ascii_cased(c) || (tool_input && matches!(c, ':' | ',' | '$' | '#'))
    };
    let best = alt
        .split(cut)
        // the spaces a tool input's text adds all follow one of its separators
        .map(|p| if tool_input { p.trim_start_matches(' ') } else { p })
        .fold("", |best, p| if p.len() > best.len() { p } else { best });
    (!best.is_empty()).then(|| best.to_string())
}

/// What to search for a regex TERM: the literals every match starts (or ends) with, each cut
/// like a literal alternative. `None` when the TERM can begin and end with too many things.
fn regex_needles(pattern: &str, tool_input: bool) -> Option<Vec<String>> {
    // parsed with case: the literals stay one per spelling, the byte search folds ASCII case
    let hir = regex_syntax::ParserBuilder::new().build().parse(pattern).ok()?;
    let mut best: Option<Vec<String>> = None;
    for kind in [ExtractKind::Prefix, ExtractKind::Suffix] {
        let seq = Extractor::new().kind(kind).extract(&hir);
        let Some(literals) = seq.literals() else { continue };
        if literals.is_empty() || literals.len() > MAX_REGEX_LITERALS {
            continue;
        }
        let Some(mut needles) = literals.iter().map(|l| needle(&String::from_utf8_lossy(l.as_bytes()), tool_input)).collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        needles.iter_mut().for_each(|n| n.make_ascii_lowercase());
        needles.sort();
        needles.dedup();
        let shortest = |ns: &[String]| ns.iter().map(String::len).min().unwrap_or(0);
        if best.as_ref().is_none_or(|b| shortest(&needles) > shortest(b)) {
            best = Some(needles);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(t: &[&str]) -> Query {
        Query::parse(&t.iter().map(|s| s.to_string()).collect::<Vec<_>>(), false, false).unwrap()
    }

    fn re(t: &str) -> Query {
        Query::parse(&[t.to_string()], true, false).unwrap()
    }

    #[test]
    fn alternatives_are_cut_at_what_json_escapes_and_at_letters_with_case() {
        assert_eq!(needle("connection refused", false).as_deref(), Some("connection refused"));
        assert_eq!(needle("连不上", false).as_deref(), Some("连不上"), "caseless characters stay");
        assert_eq!(needle("say \"hello\" now", false).as_deref(), Some("hello"));
        assert_eq!(needle("C:\\Users\\me", false).as_deref(), Some("Users"));
        assert_eq!(needle("one\nline two", false).as_deref(), Some("line two"));
        assert_eq!(needle("Über die Brücke", false).as_deref(), Some("ber die Br"));
        assert_eq!(needle("\"", false), None);
        assert_eq!(needle("ÉÀ", false), None);
    }

    #[test]
    fn tool_inputs_cut_at_the_separators_their_text_adds() {
        assert_eq!(needle("$ git push", true).as_deref(), Some("git push"));
        assert_eq!(needle("# Show the logs", true).as_deref(), Some("Show the logs"));
        assert_eq!(needle("file_path: /x/a.rs", true).as_deref(), Some("file_path"));
        assert_eq!(needle("[1, 2, 3]", true).as_deref(), Some("[1"));
        assert_eq!(needle("$ git push", false).as_deref(), Some("$ git push"));
        assert_eq!(needle("file_path:", true).as_deref(), Some("file_path"), "the raw line has `\"file_path\":`");
        assert_eq!(needle("$", true), None);
        assert_eq!(needle("#", true), None);
        assert_eq!(needle(" /x/a.rs", true).as_deref(), Some("/x/a.rs"), "a space may follow a separator in the text only");
        assert_eq!(needle("limit:20", true).as_deref(), Some("limit"));
        let p = Prefilter::new(&re("file_path:.*config"), true);
        assert!(p.line_may_match(b"{\"input\":{\"file_path\":\"/x/config.toml\"}}"));
    }

    #[test]
    fn regex_terms_search_their_literals() {
        let p = Prefilter::new(&re("connection (refused|reset)"), false);
        assert_eq!(p.open(), 0);
        assert!(p.line_may_match(b"{\"text\":\"Connection Reset by peer\"}"));
        assert!(!p.line_may_match(b"{\"text\":\"connection closed\"}"));
        let p = Prefilter::new(&re("\\d+ errors"), false);
        assert_eq!(p.open(), 0, "suffix literals when the prefix is a class");
        assert!(p.line_may_match(b"42 ERRORS"));
        assert_eq!(Prefilter::new(&re("\\w+ing"), false).pieces_of(0), 1, "the suffix `ing`");
        assert_eq!(Prefilter::new(&re("\\w+\\d"), false).open(), 1, "nothing literal on either side");
        assert_eq!(Prefilter::new(&re("über|unrelated"), false).pieces_of(0), 2, "the cased letter is cut away");
        assert_eq!(Prefilter::new(&re("a\"b"), false).open(), 0);
        assert_eq!(Prefilter::new(&re("foo("), false).open(), 0, "an invalid regex is a literal");
    }

    #[test]
    fn a_file_must_hold_every_term_even_where_they_overlap() {
        let p = Prefilter::new(&lit(&["abc", "bcd"]), false);
        assert!(p.file_may_match(b"xxabcdxx"));
        assert!(!p.file_may_match(b"xxabcxx"));
        let p = Prefilter::new(&lit(&["abc", "ÉÀ"]), false);
        assert!(p.file_may_match(b"abc"), "a TERM that cannot be checked is taken as present");
        assert!(p.line_may_match(b"nothing"), "and lets every line through");
        let p = Prefilter::new(&lit(&["ab|xy"]), false);
        assert!(p.file_may_match(b"..XY.."));
        assert!(!p.line_may_match(b"a b x y"));
    }
}
