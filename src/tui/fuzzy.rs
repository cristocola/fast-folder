//! Fuzzy matching, behind one small door.
//!
//! `nucleo-matcher` (the matcher inside helix) does the work. This wrapper
//! owns the `Matcher` — it carries scratch buffers and is reused across calls —
//! and hands back what the frames need: a score to rank by and the character
//! indices to highlight.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};

/// A match: how good, and which characters of the haystack matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub score: u32,
    /// Character (not byte) indices into the haystack, ascending.
    pub indices: Vec<u32>,
}

pub struct Fuzzy {
    matcher: Matcher,
    scratch: Vec<u32>,
}

impl Default for Fuzzy {
    fn default() -> Self {
        Self::new()
    }
}

impl Fuzzy {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            scratch: Vec::new(),
        }
    }

    /// Parse a query. Every whitespace-separated word must match somewhere;
    /// case is ignored and accents are folded, so `lullaby` finds `Lullabÿ`.
    /// Plain fuzzy atoms — none of fzf's `^`/`$`/`'` operators, which a project
    /// name could legitimately start with.
    pub fn pattern(query: &str) -> Pattern {
        Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        )
    }

    /// The haystack form a pattern is matched against. Built once per row and
    /// kept, because the conversion is the expensive half of a match.
    pub fn haystack(text: &str) -> Utf32String {
        Utf32String::from(text)
    }

    pub fn score(&mut self, pattern: &Pattern, haystack: &Utf32String) -> Option<u32> {
        pattern.score(haystack.slice(..), &mut self.matcher)
    }

    pub fn hit(&mut self, pattern: &Pattern, haystack: &Utf32String) -> Option<Hit> {
        self.scratch.clear();
        let score = pattern.indices(haystack.slice(..), &mut self.matcher, &mut self.scratch)?;
        let mut indices = self.scratch.clone();
        indices.sort_unstable();
        indices.dedup();
        Some(Hit { score, indices })
    }

    /// Rank `items` against `query`, best first. An empty query keeps every
    /// item in its original order with no highlights.
    pub fn rank<T>(&mut self, query: &str, items: Vec<(T, String)>) -> Vec<(T, Hit)> {
        if query.trim().is_empty() {
            return items
                .into_iter()
                .map(|(item, _)| {
                    (
                        item,
                        Hit {
                            score: 0,
                            indices: Vec::new(),
                        },
                    )
                })
                .collect();
        }
        let pattern = Self::pattern(query);
        let mut ranked: Vec<(usize, T, Hit)> = items
            .into_iter()
            .enumerate()
            .filter_map(|(position, (item, text))| {
                let haystack = Self::haystack(&text);
                self.hit(&pattern, &haystack)
                    .map(|hit| (position, item, hit))
            })
            .collect();
        // Best score first; ties keep declaration order, so a list that was
        // meaningful before the query stays meaningful under it.
        ranked.sort_by(|a, b| b.2.score.cmp(&a.2.score).then(a.0.cmp(&b.0)));
        ranked
            .into_iter()
            .map(|(_, item, hit)| (item, hit))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Fuzzy;

    #[test]
    fn a_query_ranks_the_closer_name_first_and_says_which_chars_hit() {
        let mut fuzzy = Fuzzy::new();
        let ranked = fuzzy.rank(
            "open",
            vec![
                ("terminal", "Open terminal here".to_string()),
                ("folder", "Open project folder".to_string()),
                ("copy", "Copy path".to_string()),
            ],
        );
        let names: Vec<&str> = ranked.iter().map(|(item, _)| *item).collect();
        assert!(names.contains(&"terminal") && names.contains(&"folder"));
        assert!(!names.contains(&"copy"), "no o-p-e-n in `Copy path`");
        assert_eq!(ranked[0].1.indices, vec![0, 1, 2, 3]);
    }

    #[test]
    fn an_empty_query_keeps_the_order() {
        let mut fuzzy = Fuzzy::new();
        let ranked = fuzzy.rank("", vec![(1, "b".to_string()), (2, "a".to_string())]);
        assert_eq!(
            ranked.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn case_and_accents_are_folded() {
        let mut fuzzy = Fuzzy::new();
        let pattern = Fuzzy::pattern("lullaby");
        assert!(
            fuzzy
                .score(&pattern, &Fuzzy::haystack("Lullabÿ_Remix"))
                .is_some()
        );
    }
}
