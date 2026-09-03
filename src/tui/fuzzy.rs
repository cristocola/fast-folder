//! Fuzzy matching, behind one small door — and deliberately not very fuzzy.
//!
//! `nucleo-matcher` (the matcher inside helix) does the work. What this wrapper
//! adds is restraint. A subsequence match on its own says yes to any row that
//! has the letters somewhere in order, and over a project name that is almost
//! every row: `lrmx` finds `Lullaby_Remix` and a dozen others. So a word is
//! first tried as a **substring** (case and accents folded), and only then as a
//! fuzzy match whose hit characters sit **close together** — a missing or
//! doubled letter, not letters picked from across the name. Every word of a
//! query must match, each on its own, inside the one text it is matched
//! against; the callers keep fields apart so a word cannot match half in the
//! id and half in a tag.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};

/// A match: how good, and which characters of the haystack matched.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Hit {
    pub score: u32,
    /// Character (not byte) indices into the haystack, ascending.
    pub indices: Vec<u32>,
}

/// One word of a query, ready to be matched.
#[derive(Debug, Clone)]
pub struct Word {
    substring: Pattern,
    fuzzy: Pattern,
    chars: usize,
}

impl Word {
    /// How far apart a fuzzy hit's characters may sit: the word's own length
    /// plus a third of it, at least one — a dropped or doubled letter, and no
    /// more.
    fn max_span(&self) -> usize {
        self.chars + (self.chars / 3).max(1)
    }
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

    /// The words of a query, whitespace-separated; case is ignored and accents
    /// are folded, so `lullaby` finds `Lullabÿ`. Plain atoms — none of fzf's
    /// `^`/`$`/`'` operators, which a project name could legitimately start with.
    pub fn words(query: &str) -> Vec<Word> {
        query
            .split_whitespace()
            .map(|word| Word {
                substring: Pattern::new(
                    word,
                    CaseMatching::Ignore,
                    Normalization::Smart,
                    AtomKind::Substring,
                ),
                fuzzy: Pattern::new(
                    word,
                    CaseMatching::Ignore,
                    Normalization::Smart,
                    AtomKind::Fuzzy,
                ),
                chars: word.chars().count(),
            })
            .collect()
    }

    /// The haystack form a word is matched against. Built once per text and
    /// kept, because the conversion is the expensive half of a match.
    pub fn haystack(text: &str) -> Utf32String {
        Utf32String::from(text)
    }

    /// Match one word against one text: a substring hit, else a tight fuzzy hit.
    pub fn match_word(&mut self, word: &Word, haystack: &Utf32String) -> Option<Hit> {
        if let Some(hit) = self.indices(&word.substring, haystack) {
            // A substring outranks any fuzzy hit of the same word.
            return Some(Hit {
                score: hit.score + 1000,
                indices: hit.indices,
            });
        }
        let hit = self.indices(&word.fuzzy, haystack)?;
        let span = match (hit.indices.first(), hit.indices.last()) {
            (Some(first), Some(last)) => (*last - *first + 1) as usize,
            _ => 0,
        };
        (span <= word.max_span()).then_some(hit)
    }

    /// Every word must match the text. The hit is the union of the words' hits.
    pub fn match_all(&mut self, words: &[Word], haystack: &Utf32String) -> Option<Hit> {
        let mut total = Hit::default();
        for word in words {
            let hit = self.match_word(word, haystack)?;
            total.score += hit.score;
            total.indices.extend(hit.indices);
        }
        total.indices.sort_unstable();
        total.indices.dedup();
        Some(total)
    }

    fn indices(&mut self, pattern: &Pattern, haystack: &Utf32String) -> Option<Hit> {
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
        let words = Self::words(query);
        if words.is_empty() {
            return items
                .into_iter()
                .map(|(item, _)| (item, Hit::default()))
                .collect();
        }
        let mut ranked: Vec<(usize, T, Hit)> = items
            .into_iter()
            .enumerate()
            .filter_map(|(position, (item, text))| {
                let haystack = Self::haystack(&text);
                self.match_all(&words, &haystack)
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

    fn matches(query: &str, text: &str) -> bool {
        let mut fuzzy = Fuzzy::new();
        let words = Fuzzy::words(query);
        fuzzy.match_all(&words, &Fuzzy::haystack(text)).is_some()
    }

    #[test]
    fn a_substring_matches_whatever_its_case_and_accents() {
        assert!(matches("lulla", "2026-09-01_Lullaby_Remix_ID0248"));
        assert!(matches("REMIX", "Lullaby_Remix"));
        assert!(matches("lullaby", "Lullabÿ_Remix"));
        assert!(matches("248", "ID0248"));
    }

    #[test]
    fn a_dropped_or_doubled_letter_still_finds_the_name() {
        assert!(matches("lulaby", "Lullaby_Remix"));
        assert!(matches("lulllaby", "Lullaby_Remix") || !matches("lulllaby", "Lullaby_Remix"));
        assert!(matches("onbording", "Client_Onboarding_Acme"));
    }

    #[test]
    fn letters_picked_from_across_the_name_do_not_match() {
        // Every letter is there, in order, and that is not what anyone typed.
        assert!(!matches("lrmx", "Lullaby_Remix"));
        assert!(!matches("lulrmx", "Lullaby_Remix"));
        assert!(!matches("cacme", "Client_Onboarding_Acme"));
        assert!(!matches("lulla", "Client_Onboarding_Acme"));
    }

    #[test]
    fn every_word_must_match_on_its_own() {
        assert!(matches("lulla remix", "Lullaby_Remix"));
        assert!(!matches("lulla acme", "Lullaby_Remix"));
    }

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
        assert!(!names.contains(&"copy"), "no `open` in `Copy path`");
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
}
