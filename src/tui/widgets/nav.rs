//! The arithmetic every list shares: arrows wrap, page keys clamp, and the
//! viewport follows the selection without jumping.

/// Move `selected` by `delta`, wrapping at both ends.
pub fn wrap_step(selected: Option<usize>, len: usize, delta: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let current = selected.unwrap_or(0) as isize;
    let next = (current + delta).rem_euclid(len as isize);
    Some(next as usize)
}

/// Move `selected` by `delta`, stopping at the ends.
pub fn clamp_jump(selected: Option<usize>, len: usize, delta: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let current = selected.unwrap_or(0) as isize;
    let next = (current + delta).clamp(0, len as isize - 1);
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
    use super::{clamp_jump, viewport_offset, wrap_step};

    #[test]
    fn arrows_wrap_and_page_keys_clamp() {
        assert_eq!(wrap_step(Some(0), 3, -1), Some(2));
        assert_eq!(wrap_step(Some(2), 3, 1), Some(0));
        assert_eq!(wrap_step(None, 3, 1), Some(1));
        assert_eq!(wrap_step(Some(0), 0, 1), None);
        assert_eq!(clamp_jump(Some(1), 3, -10), Some(0));
        assert_eq!(clamp_jump(Some(1), 3, 10), Some(2));
        assert_eq!(clamp_jump(None, 0, 1), None);
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
