//! A count's plural, said once: the ending of a regular noun, and the word of
//! an irregular one.

/// `""` for one and `"s"` for any other count.
pub fn s<N: PartialEq + From<u8>>(count: N) -> &'static str {
    of(count, "", "s")
}

/// `one` for a count of one, `many` for any other.
pub fn of<N: PartialEq + From<u8>>(
    count: N,
    one: &'static str,
    many: &'static str,
) -> &'static str {
    if count == N::from(1) { one } else { many }
}

#[cfg(test)]
mod tests {
    use super::{of, s};

    #[test]
    fn one_is_singular_and_nothing_else_is() {
        assert_eq!(
            [s(0_usize), s(1_usize), s(2_usize)],
            ["s", "", "s"],
            "0 folders, 1 folder, 2 folders"
        );
        assert_eq!(s(1_u64), "");
        assert_eq!(of(1_usize, "entry", "entries"), "entry");
        assert_eq!(of(7_u32, "was", "were"), "were");
    }
}
