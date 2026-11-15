//! The TERMs of a session search (`asm search`, `asm transcript --grep`): each TERM is one
//! concept, `|` separates ways of saying it. Literal TERMs are case-insensitive text; with
//! `regex`, each TERM is one regular expression.

use regex::{Regex, RegexBuilder};

use super::Matcher;

/// Bit `i` stands for TERM `i`; a query has at most 64 TERMs.
pub type TermMask = u64;

pub const MAX_TERMS: usize = 64;

/// One concept.
#[derive(Clone, Debug)]
pub struct Term {
    /// Literal: the alternatives as given, surrounding spaces and empty ones dropped. Regex: the
    /// whole TERM as one entry.
    pub alternatives: Vec<String>,
    /// Regex mode: the TERM did not compile and is matched as literal text.
    pub fallback: bool,
    re: Regex,
}

#[derive(Clone, Debug)]
pub struct Query {
    terms: Vec<Term>,
    pub regex: bool,
    pub same_message: bool,
    union: Matcher,
}

/// Invalid TERMs (exit 2). Indexes count from 0, messages from 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryError {
    NoTerms,
    TooManyTerms,
    /// Nothing left once empty alternatives are dropped.
    EmptyTerm(usize),
    /// A regular expression that can match the empty string, so it would match everything.
    MatchesEmpty(usize),
    /// More than the regex engine compiles (10 MiB of program): one huge TERM, or several large
    /// regexes that each compile but not all together.
    TooLarge,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::NoTerms => write!(f, "no TERM given"),
            QueryError::TooManyTerms => write!(f, "at most {MAX_TERMS} TERMs"),
            QueryError::EmptyTerm(i) => write!(f, "term {} has nothing to match", i + 1),
            QueryError::MatchesEmpty(i) => write!(f, "term {} can match empty text, so it would match everything", i + 1),
            QueryError::TooLarge => write!(f, "the TERMs are too large to match together; use fewer or smaller ones"),
        }
    }
}

impl std::error::Error for QueryError {}

impl Query {
    pub fn parse(terms: &[String], regex: bool, same_message: bool) -> Result<Query, QueryError> {
        if terms.is_empty() {
            return Err(QueryError::NoTerms);
        }
        if terms.len() > MAX_TERMS {
            return Err(QueryError::TooManyTerms);
        }
        let terms: Vec<Term> =
            terms.iter().enumerate().map(|(i, t)| if regex { regex_term(i, t) } else { literal_term(i, t) }).collect::<Result<_, _>>()?;
        let union = union_matcher(&terms, regex).ok_or(QueryError::TooLarge)?;
        Ok(Query { terms, regex, same_message, union })
    }

    pub fn terms(&self) -> &[Term] {
        &self.terms
    }

    pub fn len(&self) -> usize {
        self.terms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Every TERM's bit set.
    pub fn all(&self) -> TermMask {
        if self.terms.len() == MAX_TERMS { u64::MAX } else { (1 << self.terms.len()) - 1 }
    }

    /// Indexes of the regex TERMs that did not compile.
    pub fn regex_fallback(&self) -> Vec<usize> {
        self.terms.iter().enumerate().filter(|(_, t)| t.fallback).map(|(i, _)| i).collect()
    }

    /// Highlights a match of any TERM.
    pub fn matcher(&self) -> &Matcher {
        &self.union
    }

    /// The TERMs that occur in `text`.
    pub fn mask(&self, text: &str) -> TermMask {
        let mut mask = 0;
        for (i, t) in self.terms.iter().enumerate() {
            if t.re.is_match(text) {
                mask |= 1 << i;
            }
        }
        mask
    }

    /// Indexes of the TERMs that occur in `text`.
    pub fn terms_in(&self, text: &str) -> Vec<usize> {
        indexes(self.mask(text))
    }
}

/// Whether `s` has a non-ASCII character with case: matching it case-insensitively needs Unicode
/// case folding, which neither SQL `LIKE` nor an ASCII-only byte search does.
pub(crate) fn has_non_ascii_case(s: &str) -> bool {
    s.chars().any(non_ascii_cased)
}

pub(crate) fn non_ascii_cased(c: char) -> bool {
    !c.is_ascii() && (c.is_lowercase() || c.is_uppercase() || !c.to_lowercase().eq(c.to_uppercase()))
}

/// The set bits of a mask, lowest first.
pub fn indexes(mask: TermMask) -> Vec<usize> {
    (0..MAX_TERMS).filter(|i| mask & (1 << i) != 0).collect()
}

/// Literal mode: `|` separates alternatives; the spaces around them do not count.
fn literal_term(i: usize, term: &str) -> Result<Term, QueryError> {
    let alternatives: Vec<String> = term.split('|').map(str::trim).filter(|a| !a.is_empty()).map(String::from).collect();
    if alternatives.is_empty() {
        return Err(QueryError::EmptyTerm(i));
    }
    let re = build(&alternation(alternatives.iter().map(String::as_str))).map_err(|_| QueryError::TooLarge)?;
    Ok(Term { alternatives, fallback: false, re })
}

/// Regex mode: the TERM as given; one that does not compile is literal text (not split at `|`).
fn regex_term(i: usize, term: &str) -> Result<Term, QueryError> {
    match build(term) {
        Ok(re) => {
            if can_match_empty(term) {
                return Err(QueryError::MatchesEmpty(i));
            }
            Ok(Term { alternatives: vec![term.to_string()], fallback: false, re })
        }
        Err(_) => {
            let re = build(&regex::escape(term)).map_err(|_| QueryError::TooLarge)?;
            Ok(Term { alternatives: vec![term.to_string()], fallback: true, re })
        }
    }
}

/// Zero-width matches count (`\b`, `^`): such a TERM matches every text.
fn can_match_empty(pattern: &str) -> bool {
    regex_syntax::ParserBuilder::new()
        .case_insensitive(true)
        .build()
        .parse(pattern)
        .map(|hir| hir.properties().minimum_len() == Some(0))
        .unwrap_or(false)
}

fn build(pattern: &str) -> Result<Regex, regex::Error> {
    RegexBuilder::new(pattern).case_insensitive(true).build()
}

/// Escaped alternatives, longest first so that a longer one wins over its own prefix.
fn alternation<'a>(alternatives: impl Iterator<Item = &'a str>) -> String {
    let mut alts: Vec<&str> = alternatives.collect();
    alts.sort_by_key(|a| std::cmp::Reverse(a.chars().count()));
    alts.iter().map(|a| regex::escape(a)).collect::<Vec<_>>().join("|")
}

/// One regex for every TERM, to highlight them; `None` when it is too large to compile (each
/// TERM alone did).
fn union_matcher(terms: &[Term], regex: bool) -> Option<Matcher> {
    let pattern = if regex {
        terms.iter().map(|t| format!("(?:{})", t.re.as_str())).collect::<Vec<_>>().join("|")
    } else {
        alternation(terms.iter().flat_map(|t| t.alternatives.iter().map(String::as_str)))
    };
    let re = build(&pattern).ok()?;
    Some(Matcher { pattern: format!("(?i:{})", re.as_str()), re })
}
