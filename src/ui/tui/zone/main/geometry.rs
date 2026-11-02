//! The input container's arithmetic: border width, height cap, scroll, cursor.

/// Cells the frame takes: `| ` and ` |`, or `+-` and `-+`.
pub const BORDER_COLS: usize = 4;

/// The container's floor: the statusline row and the bottom edge.
pub const MIN_CONTAINER_ROWS: usize = 2;

/// Text width inside the frame.
pub fn inner_width(term_w: u16) -> usize {
    (term_w as usize).saturating_sub(BORDER_COLS).max(1)
}

/// The input container's row ceiling: a fifth of the terminal, at least two rows.
pub fn max_container_height(term_h: u16) -> usize {
    let fifth = (term_h as usize / 5).max(MIN_CONTAINER_ROWS);
    fifth.min(term_h as usize)
}

/// The container's height: the content, capped at a fifth of the rows available.
pub fn container_height(term_h: u16, total_lines: usize) -> usize {
    let avail = term_h as usize;
    (total_lines + 1)
        .min(max_container_height(term_h))
        .min(avail)
        .max(MIN_CONTAINER_ROWS.min(avail))
}

/// The largest first visible row: the last covered row is `first + h - 2`.
pub fn max_first(total_lines: usize, container_h: usize) -> usize {
    (total_lines + 1).saturating_sub(container_h)
}

/// Keep the cursor visible with the smallest move; `prev` is required to tell a
/// cursor that was already in view from one that just arrived.
pub fn adjust_scroll(
    prev: usize,
    total_lines: usize,
    container_h: usize,
    cursor_row: usize,
) -> usize {
    let max_first = max_first(total_lines, container_h);
    let first = prev.min(max_first);
    let last_row_in_view = first + container_h.saturating_sub(2);
    if cursor_row < first {
        cursor_row.min(max_first)
    } else if cursor_row > last_row_in_view {
        (cursor_row - container_h.saturating_sub(2)).min(max_first)
    } else {
        first
    }
}

/// Container coordinates to terminal coordinates, clamped inside the container,
/// above the bottom edge, and out of the reserved strip.
pub fn cursor_position(
    term: (u16, u16),
    container_y: u16,
    container_rows: u16,
    reserved_rows: u16,
    row: u16,
    col: u16,
) -> (u16, u16) {
    let (cols, rows) = term;
    let last_allowed = rows.saturating_sub(reserved_rows).saturating_sub(1);
    let in_container = row.min(container_rows.saturating_sub(1));
    let y = container_y.saturating_add(in_container).min(last_allowed);
    let x = col.min(cols.saturating_sub(1));
    (x, y)
}
