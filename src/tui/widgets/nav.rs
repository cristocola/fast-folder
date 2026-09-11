//! The arithmetic every list shares: the arrows and the page keys stop at
//! the ends, and the viewport follows the selection without jumping.
//!
//! **A list does not wrap.** It did, from the first row up to the last and
//! from the last down to the first, and a cursor that leaves the bottom of a
//! table and reappears at the top reads as the cursor escaping — one key too
//! many at the end of a long list and you are somewhere else with nothing to
//! say why. Stopping is what a scrollbar promises, and every list here has
//! one or behaves as if it did. The one thing that still cycles is a
//! *value*: a form's choice steps through its options with `←`/`→` and has
//! to come round, and a form's Tab ring has to reach its first field from its
//! last. Those are `cycle`, and they are not lists.

/// Move `selected` by `delta`, stopping at the ends.
///
/// `saturating_add`, because the jump keys pass `isize::MIN`/`isize::MAX` for
/// the ends, and `current + isize::MAX` is an overflow in debug for any
/// `current` above zero.
pub fn step(selected: Option<usize>, len: usize, delta: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let current = selected.unwrap_or(0) as isize;
    let next = current.saturating_add(delta).clamp(0, len as isize - 1);
    Some(next as usize)
}

/// Move `selected` by `delta` through a ring of values, coming round at the
/// ends. For a value selector or a focus ring — never for a list.
pub fn cycle(selected: Option<usize>, len: usize, delta: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let current = selected.unwrap_or(0) as isize;
    let next = (current + delta).rem_euclid(len as isize);
    Some(next as usize)
}

/// The first visible row, given the one it showed last time: unchanged while
/// the selection is inside the window, moved by the minimum otherwise.
pub fn viewport_offset(offset: usize, selected: Option<usize>, len: usize, rows: usize) -> usize {
    if rows == 0 || len == 0 {
        return 0;
    }
    let max_offset = len.saturating_sub(rows);
    let offset = offset.min(max_offset);
    let Some(selected) = selected else {
        return offset;
    };
    if selected < offset {
        selected
    } else if selected >= offset + rows {
        selected + 1 - rows
    } else {
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::{cycle, step, viewport_offset};

    #[test]
    fn a_list_stops_at_both_ends_and_a_value_comes_round() {
        assert_eq!(step(Some(0), 3, -1), Some(0), "up from the top stays");
        assert_eq!(step(Some(2), 3, 1), Some(2), "down from the bottom stays");
        assert_eq!(step(None, 3, 1), Some(1));
        assert_eq!(step(Some(0), 0, 1), None);
        assert_eq!(step(Some(1), 3, -10), Some(0));
        assert_eq!(step(Some(1), 3, 10), Some(2));
        // The ends are asked for with the extreme deltas; that is not an
        // overflow.
        assert_eq!(step(Some(3), 5, isize::MAX), Some(4));
        assert_eq!(step(Some(3), 5, isize::MIN), Some(0));
        assert_eq!(cycle(Some(0), 3, -1), Some(2));
        assert_eq!(cycle(Some(2), 3, 1), Some(0));
        assert_eq!(cycle(None, 0, 1), None);
    }

    #[test]
    fn the_window_does_not_move_while_the_selection_is_visible() {
        assert_eq!(viewport_offset(3, Some(5), 20, 5), 3);
        assert_eq!(viewport_offset(3, Some(7), 20, 5), 3);
        assert_eq!(viewport_offset(3, Some(8), 20, 5), 4);
        assert_eq!(viewport_offset(3, Some(1), 20, 5), 1);
    }

    #[test]
    fn the_window_clamps_to_the_ends() {
        assert_eq!(viewport_offset(50, Some(19), 20, 5), 15);
        assert_eq!(viewport_offset(0, None, 3, 5), 0);
        assert_eq!(viewport_offset(9, Some(0), 20, 0), 0);
    }
}
