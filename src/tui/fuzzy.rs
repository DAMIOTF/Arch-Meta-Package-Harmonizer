//! Fuzzy filtering backed by `nucleo-matcher`.
//!
//! A single `FuzzyFilter` owns the matcher scratch space and is reused for every
//! candidate in one filtering pass. Filtering is only constructed when the query
//! is non-empty, so the normal cost of the TUI is unchanged.

use nucleo_matcher::Utf32Str;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

pub(crate) struct FuzzyFilter {
    pattern: Pattern,
    matcher: Matcher,
    buffer: Vec<char>,
}

impl FuzzyFilter {
    pub(crate) fn new(query: &str) -> Option<Self> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }

        Some(Self {
            // `Ignore` keeps `/` friendly: `RUST` and `rust` behave the same.
            pattern: Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart),
            matcher: Matcher::new(Config::DEFAULT),
            buffer: Vec::new(),
        })
    }

    pub(crate) fn score(&mut self, haystack: &str) -> Option<u32> {
        let Self {
            pattern,
            matcher,
            buffer,
        } = self;
        pattern.score(Utf32Str::new(haystack, buffer), matcher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_contiguous_query_matches() {
        let mut filter = FuzzyFilter::new("rgrp").expect("query");
        assert!(filter.score("ripgrep").is_some());
        assert!(filter.score("some random group").is_some());
    }

    #[test]
    fn case_is_ignored() {
        let mut filter = FuzzyFilter::new("RiPgReP").expect("query");
        assert!(filter.score("ripgrep").is_some());
    }

    #[test]
    fn unrelated_values_do_not_match() {
        let mut filter = FuzzyFilter::new("zzzz").expect("query");
        assert!(filter.score("hyprland").is_none());
        assert!(filter.score("rustup").is_none());
    }

    #[test]
    fn empty_query_does_not_build_a_filter() {
        assert!(FuzzyFilter::new("").is_none());
        assert!(FuzzyFilter::new("   ").is_none());
    }
}
