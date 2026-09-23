//! Vertical layout math — splits the terminal height into "history area + input container". Pure arithmetic.
//!
//! Layout convention (anchored bottom-up; the input container sits flush at the bottom):
//! ```text
//! ┌───────────────────────────────────┐
//! │ history (row 1.., fills the rest) │
//! ├───────────────────────────────────┤  ← container top: status bar (1 row)
//! │ | input rows 1..n-1 |             │  ← only when the input spans >1 row
//! ├───────────────────────────────────┤  ← container bottom: +-{last row}-+
//! └───────────────────────────────────┘
//! ```
//!
//! User constraint: the input container takes at most 1/4 of the terminal height.

use crate::tui::text;

/// Horizontal cells consumed by the border: the left `| ` or `+-` takes 2, the right ` |` or `-+` takes 2.
///
/// Single source of truth for border width in this project: every place that carves the
/// border out of a width goes through it, so the numbers can never drift apart.
pub const BORDER_COLS: usize = 4;

/// **Default** height of the bottom reserved area (idle state: 1 blank row).
///
/// The reserved area is a generic pop-up region: completion candidates and hint bars
/// are drawn here. It scales with the content: callers pass `reserved_want` (rows the
/// content needs) and `reserved_max` (allowed ceiling); the idle state passes 1/1,
/// which is just the blank row.
pub const RESERVED_IDLE: u16 = 1;

/// Wrap width for the input area: terminal width minus the border. Must match the layout in `components::input`.
pub fn inner_width(term_w: u16) -> usize {
    (term_w as usize).saturating_sub(BORDER_COLS).max(1)
}

/// Height cap for the input container: 1/4 of the terminal height, and at least 2 rows (status bar + bottom edge).
pub fn max_container_height(term_height: u16) -> usize {
    let quarter = (term_height as usize / 4).max(2);
    quarter.min(term_height as usize)
}

/// Computed layout dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Height of the history (chat) area.
    pub chat_height: u16,
    /// Input container height (includes 1 status-bar row and 1 bottom-edge row).
    pub container_height: u16,
    /// Number of `| ... |` rows inside the container (= container height - 2).
    pub body_rows: usize,
    /// First wrapped row covered by the viewport.
    pub first_visible: usize,
    /// Height the reserved area actually occupies this frame (1 = idle blank row, >1 = pop-up content).
    pub reserved_height: u16,
}

impl Layout {
    /// Number of wrapped rows the viewport covers (`|` rows plus the row that slides into the bottom edge = container height - 1).
    #[inline]
    pub fn visible_rows(&self) -> usize {
        self.body_rows + 1
    }

    /// Row index (0-based) of the input container's top edge in the terminal.
    ///
    /// **Always use this to place the hardware cursor** — do not subtract by hand elsewhere:
    /// the terminal's bottom row is the reserved area and the container sits above it.
    /// Skipping that subtraction pushes the cursor into the reserved area.
    #[inline]
    pub fn container_y(&self, term_h: u16) -> u16 {
        (term_h as usize)
            .saturating_sub(self.reserved_height as usize)
            .saturating_sub(self.container_height as usize) as u16
    }

    /// **Absolute** row of the hardware cursor. The only place in this project allowed to compute it.
    ///
    /// `row` is the row offset relative to the container top, as handed over by `view::draw`.
    ///
    /// Why clamp here again instead of trusting `view`:
    /// ratatui / crossterm perform **no bounds checking** —
    /// `Terminal::set_cursor_position` feeds the coordinates straight into `MoveTo(x, y)`
    /// (see `apply_buffer_with_cursor` in ratatui-core `terminal/render.rs`).
    /// What an out-of-range coordinate does is up to the terminal itself; we have no
    /// control over it. So this is the last and only gate. The returned row is guaranteed to:
    ///
    /// 1. land inside the input container (`row` may overshoot when the container is short);
    /// 2. stay above the terminal's bottom edge;
    /// 3. **never enter the bottom reserved area**.
    ///
    /// Note that `row` is an **offset relative to the container top**, not an absolute row:
    /// the valid range inside the container is `0..container_height`; anything beyond is clamped.
    /// Never feed this function's return value back in as `row` — that is an absolute row,
    /// a different semantic, and doing so nudges the cursor one row further up.
    #[inline]
    pub fn cursor_y(&self, term_h: u16, row: u16) -> u16 {
        let row = row.min(self.container_height.saturating_sub(1));
        let y = self.container_y(term_h).saturating_add(row);
        // The reserved area starts at `term_h - reserved_height`; the cursor may reach at most the row above it
        let last_allowed = term_h.saturating_sub(self.reserved_height).saturating_sub(1);
        y.min(last_allowed)
    }
}

/// Container height: wants to fit all content (status bar + rows + bottom edge = total rows + 1),
/// but never exceeds 1/4 of the terminal height and **never exceeds the available height**.
///
/// `term_height` must be the height available *after* carving out the reserved area.
///
/// The floor is `2.min(avail)`, deliberately not a trailing `.max(2)`: a trailing
/// `.max(2)` would override the `.min(avail)` before it — on a 1-row terminal the
/// container would come out 2 rows tall, taller than the screen, pushing the hardware
/// cursor off the terminal. With `2.min(avail)`: 2 rows (status bar + bottom edge) when
/// there is room, otherwise as much as fits.
pub fn container_height(term_height: u16, total_lines: usize) -> usize {
    let avail = term_height as usize;
    (total_lines + 1)
        .min(max_container_height(term_height))
        .min(avail)
        .max(2.min(avail))
}

/// Largest start row the viewport can scroll to.
///
/// With viewport height `h`, the covered wrapped rows are `[first, first + h - 2]`
/// (the last cell slides into the bottom edge), and the covered last row must not go
/// past the text's final row: `first + h - 2 <= total - 1`.
pub fn max_first(total_lines: usize, container_h: usize) -> usize {
    (total_lines + 1).saturating_sub(container_h)
}

/// Given the terminal size and the wrap result, compute each region's dimensions
/// (the viewport start is taken from `scroll` and clamped into range).
///
/// Height only: horizontal wrapping is the caller's job, already done via `inner_width`.
///
/// `reserved_want` / `reserved_max`: the height the reserved area wants and its allowed
/// ceiling (how many rows the pop-up content needs, and the cap). The idle state passes
/// (RESERVED_IDLE, RESERVED_IDLE).
pub fn compute(
    term_h: u16,
    wrapped: &text::Wrapped,
    scroll: usize,
    reserved_want: u16,
    reserved_max: u16,
) -> Layout {
    let total_lines = wrapped.len();
    let reserved = reserved_want.min(reserved_max).max(1).min(term_h);
    // Carve out the reserved area first; whatever is left goes to the input container and the chat area
    let avail = (term_h as usize).saturating_sub(reserved as usize);
    let ch = container_height(avail as u16, total_lines);
    let body_rows = ch.saturating_sub(2);

    Layout {
        chat_height: avail.saturating_sub(ch) as u16,
        container_height: ch as u16,
        body_rows,
        first_visible: scroll.min(max_first(total_lines, ch)),
        reserved_height: reserved,
    }
}

/// Viewport scrolling: keep the cursor inside the viewport with the **minimum displacement**.
///
/// The standard text-editor approach — do not scroll while the cursor moves within the
/// viewport (the picture stays stable); scroll up only when the cursor hits the top
/// edge, scroll down only when it hits the bottom edge.
/// This is also the requested behavior: once the input overflows, earlier rows collapse
/// out of view, and moving the cursor back up scrolls them back into view.
///
/// Why `prev` must be passed: a pure function cannot tell "cursor in the middle of the
/// viewport" from "just scrolled in from below" and would degenerate into a cursor that
/// always hugs an edge.
pub fn adjust_scroll(
    prev: usize,
    total_lines: usize,
    container_h: usize,
    cursor_row: usize,
) -> usize {
    let max_first = max_first(total_lines, container_h);
    let first = prev.min(max_first);
    // The bottom-edge row also belongs to the viewport, so the last row available to the cursor is first + (h-2)
    let last_row_in_view = first + container_h.saturating_sub(2);
    if cursor_row < first {
        cursor_row.min(max_first)
    } else if cursor_row > last_row_in_view {
        (cursor_row - container_h.saturating_sub(2)).min(max_first)
    } else {
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Cursor-constraint invariants (exhaustive over all sizes) ----
    //
    // The contract: the cursor stays inside the input area, never wanders into the
    // chat area, and never touches the bottom reserved area.
    //
    // ratatui / crossterm perform **no bounds checking**, so this invariant rests
    // entirely on our own arithmetic. Enumerate it exhaustively and pin it down.

    /// Given a terminal size and reserved height, the largest legal row the cursor can reach.
    fn last_legal_row(term_h: u16, reserved: u16) -> u16 {
        term_h.saturating_sub(reserved).saturating_sub(1)
    }

    #[test]
    fn cursor_never_leaves_the_container_across_all_sizes() {
        for term_h in 1u16..=80 {
            for total_lines in [1usize, 2, 3, 5, 12, 40, 200] {
                for scroll in [0usize, 1, 6, 99, 9999] {
                    let w = text::wrap(&"x".repeat(total_lines * 3), 20);
                    let l = compute(term_h, &w, scroll, RESERVED_IDLE, RESERVED_IDLE);
                    let visible = l.visible_rows();
                    // Row offsets `view::draw` may hand out
                    for row in 0..=(visible as u16 + 2) {
                        let y = l.cursor_y(term_h, row);
                        assert!(
                            y < term_h,
                            "光标越出终端: term_h={term_h} total={total_lines} \
                             scroll={scroll} row={row} → y={y}"
                        );
                        if term_h > l.reserved_height {
                            assert!(
                                y <= last_legal_row(term_h, l.reserved_height),
                                "光标踩进保留区: term_h={term_h} total={total_lines} \
                                 scroll={scroll} row={row} → y={y}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn cursor_y_maps_offsets_inside_the_container() {
        // Legal offsets inside the container should land at "container top + offset" unchanged;
        // offsets beyond that clamp to the container's last row.
        for term_h in 3u16..=60 {
            let w = text::wrap(&"z".repeat(200), 10);
            let l = compute(term_h, &w, 0, RESERVED_IDLE, RESERVED_IDLE);
            let ch = l.container_height;
            let top = l.container_y(term_h);
            for row in 0..ch {
                assert_eq!(
                    l.cursor_y(term_h, row),
                    top + row,
                    "term_h={term_h} row={row} 应在容器内"
                );
            }
            // Out-of-range offset → clamped to the container's last row (unless the reserved area blocks it)
            let clamped = l.cursor_y(term_h, ch + 5);
            assert!(
                clamped < top + ch && clamped < term_h,
                "term_h={term_h}: 越界偏移应钳在容器内"
            );
        }
    }

    #[test]
    fn container_never_exceeds_viewport() {
        // Regression guard: a trailing `.max(2)` would override the `.min(avail)` before it —
        // on a 1-row terminal the container height would come out 2, taller than the screen,
        // pushing the cursor out of the terminal.
        for avail in 0u16..=60 {
            for total in [1usize, 5, 500] {
                let ch = container_height(avail, total);
                assert!(
                    ch <= avail as usize,
                    "容器高 {ch} 超过可用高度 {avail}（total={total}）"
                );
            }
        }
    }

    #[test]
    fn tiny_terminal_still_puts_cursor_inside() {
        // Extreme sizes must neither panic nor go out of bounds
        for term_h in 0u16..=3 {
            let w = text::wrap("abc", 20);
            let l = compute(term_h, &w, 0, RESERVED_IDLE, RESERVED_IDLE);
            let y = l.cursor_y(term_h, 0);
            if term_h > 0 {
                assert!(y < term_h, "term_h={term_h} → y={y}");
            } else {
                assert_eq!(y, 0);
            }
        }
    }

    #[test]
    fn cursor_stays_out_of_reserved_row() {
        // The reserved area is 1 row: the cursor reaches at most term_h - 2
        for term_h in 2u16..=40 {
            let w = text::wrap(&"y".repeat(200), 20);
            let l = compute(term_h, &w, 9999, RESERVED_IDLE, RESERVED_IDLE);
            let y = l.cursor_y(term_h, u16::MAX);
            assert_eq!(y, term_h - 2, "term_h={term_h}: 光标应停在保留区上一行");
        }
    }

    #[test]
    fn single_line_input_uses_two_rows() {
        let w = text::wrap("hi", 40);
        let l = compute(24, &w, 0, RESERVED_IDLE, RESERVED_IDLE);
        assert_eq!(l.container_height, 2); // status bar + bottom edge
        assert_eq!(l.body_rows, 0);
        assert_eq!(l.visible_rows(), 1);
        assert_eq!(l.chat_height, 21, "24 - 容器2 - 保留区1");
    }

    #[test]
    fn multiline_input_grows() {
        let w = text::wrap("a\nb\nc", 40);
        let l = compute(24, &w, 0, RESERVED_IDLE, RESERVED_IDLE);
        assert_eq!(l.container_height, 4); // status bar + 2 `|` rows + bottom edge
        assert_eq!(l.body_rows, 2);
        assert_eq!(l.visible_rows(), 3);
        assert_eq!(l.first_visible, 0);
    }

    #[test]
    fn container_capped_at_quarter_height() {
        // Available height 23 (24 minus the reserved row) → cap 23/4 = 5 rows
        let tall: String = (0..100).map(|i| format!("line{i}\n")).collect();
        let w = text::wrap(&tall, 40);
        let l = compute(24, &w, max_first(w.len(), 5), RESERVED_IDLE, RESERVED_IDLE);
        assert_eq!(l.container_height, 5);
        assert_eq!(l.body_rows, 3);
        // Scrolled to the bottom: the viewport covers the last 4 rows (last row slides into the bottom edge)
        assert_eq!(l.first_visible, w.len() - 4);
        assert_eq!(
            l.first_visible + l.visible_rows() - 1,
            w.len() - 1,
            "视口最后一行应正好落在文本末行"
        );
        assert_eq!(l.chat_height, 18);
    }

    #[test]
    fn cap_scales_with_terminal() {
        assert_eq!(max_container_height(24), 6);
        assert_eq!(max_container_height(40), 10);
        assert_eq!(max_container_height(8), 2); // small terminals still get at least 2 rows
        assert_eq!(max_container_height(4), 2);
    }

    #[test]
    fn long_wrapped_line_respects_cap() {
        // Width 4, 100 full-width chars → 50 visual rows
        let s: String = "中".repeat(100);
        let w = text::wrap(&s, 4);
        assert_eq!(w.len(), 50);
        let l = compute(24, &w, max_first(w.len(), 5), RESERVED_IDLE, RESERVED_IDLE);
        assert_eq!(l.container_height, 5);
        assert_eq!(l.first_visible, 46, "50 行、视口 4 行 → 起点 46");
        assert_eq!(l.first_visible + l.visible_rows() - 1, 49);
    }

    #[test]
    fn compute_clamps_scroll_into_range() {
        let w = text::wrap("a\nb\nc", 40);
        // When the content fits, any scroll clamps to 0
        let l = compute(24, &w, 999, RESERVED_IDLE, RESERVED_IDLE);
        assert_eq!(l.first_visible, 0);
        assert_eq!(l.first_visible + l.visible_rows() - 1, 2);
    }

    // ---- Viewport scrolling: core regressions for "can the cursor move back up" ----

    #[test]
    fn scroll_stays_put_while_cursor_inside_view() {
        // 40 rows, container 6 (viewport 5 rows). Viewport start 10 → covers 10..=14
        let total = 40;
        let ch = 6;
        for cursor in 10..=14 {
            assert_eq!(
                adjust_scroll(10, total, ch, cursor),
                10,
                "光标 {cursor} 已在视口内，不该滚动"
            );
        }
    }

    #[test]
    fn scroll_follows_cursor_up_over_top_edge() {
        // Viewport 10..=14, cursor moves up to row 9 → the viewport scrolls up, row 9 becomes visible
        assert_eq!(adjust_scroll(10, 40, 6, 9), 9);
        assert_eq!(adjust_scroll(10, 40, 6, 0), 0);
    }

    #[test]
    fn scroll_follows_cursor_down_over_bottom_edge() {
        // Viewport 10..=14, cursor moves down to row 15 → the viewport scrolls down
        assert_eq!(adjust_scroll(10, 40, 6, 15), 11);
        // Clamped by `max_first` at the bottom
        assert_eq!(adjust_scroll(30, 40, 6, 39), max_first(40, 6));
    }

    #[test]
    fn scroll_invariant_cursor_always_visible() {
        // Exhaustive: simulate holding ↑ / ↓ across the whole text; at every step the cursor must lie within the viewport's coverage
        let total = 30usize;
        let ch = 6usize;
        let view_rows = ch - 1; // number of visible wrapped rows
        let mut scroll = 0usize;
        // Walking down
        for cursor in 0..total {
            scroll = adjust_scroll(scroll, total, ch, cursor);
            let first = scroll;
            let last = (scroll + view_rows - 1).min(total - 1);
            assert!(
                cursor >= first && cursor <= last,
                "下移时光标不可见 cursor={cursor} first={first} last={last}"
            );
        }
        // Walking back up
        for cursor in (0..total).rev() {
            scroll = adjust_scroll(scroll, total, ch, cursor);
            let first = scroll;
            let last = (scroll + view_rows - 1).min(total - 1);
            assert!(
                cursor >= first && cursor <= last,
                "上移时光标不可见 cursor={cursor} first={first} last={last}"
            );
        }
    }

    #[test]
    fn scroll_invariant_when_content_shorter_than_view() {
        // Content shorter than one screen: the viewport always sticks to the top, so every cursor position is visible
        let total = 3usize;
        let ch = 10usize;
        let mut scroll = 0;
        for cursor in 0..total {
            scroll = adjust_scroll(scroll, total, ch, cursor);
            assert_eq!(scroll, 0);
        }
    }

    #[test]
    fn max_first_math() {
        // A viewport of height h covers h-1 rows and must not reach past total-1 → first_max = total - h + 1
        assert_eq!(max_first(40, 6), 35);
        assert_eq!(max_first(6, 6), 1);
        assert_eq!(max_first(2, 6), 0, "内容不足一屏时为 0，不 panic");
        assert_eq!(max_first(0, 6), 0);
    }

    #[test]
    fn reserved_row_is_carved_out_of_total_height() {
        let w = text::wrap("hi", 40);
        let l = compute(24, &w, 0, RESERVED_IDLE, RESERVED_IDLE);
        // Of the 24 rows, 1 goes to the reserved area: container 2 + chat 21 = 23
        assert_eq!(l.container_height, 2);
        assert_eq!(l.chat_height, 21);
        assert_eq!(
            l.chat_height as usize
                + l.container_height as usize
                + l.reserved_height as usize,
            24,
            "三段高度必须正好铺满终端"
        );
    }

    #[test]
    fn container_y_sits_above_the_reserved_row() {
        // Container top + container height must stop exactly before the reserved area,
        // otherwise the hardware cursor lands inside the reserved area.
        let w = text::wrap("hi", 40);
        let l = compute(24, &w, 0, RESERVED_IDLE, RESERVED_IDLE);
        assert_eq!(l.container_y(24), 21, "容器顶行 = 24 - 1(保留) - 2(容器)");
        assert_eq!(
            l.container_y(24) + l.container_height,
            24 - l.reserved_height,
            "容器底边应紧贴保留区上方"
        );
    }

    #[test]
    fn container_y_with_tall_input() {
        let tall: String = (0..100).map(|i| format!("line{i}\n")).collect();
        let w = text::wrap(&tall, 40);
        let l = compute(24, &w, 0, RESERVED_IDLE, RESERVED_IDLE);
        assert_eq!(l.container_height, 5);
        assert_eq!(l.container_y(24), 18, "24 - 1 - 5");
    }

    #[test]
    fn reserved_row_shrinks_tall_container_cap_too() {
        // The cap uses the *available* height: 23/4 = 5, not 24/4 = 6
        let tall: String = (0..100).map(|i| format!("line{i}\n")).collect();
        let w = text::wrap(&tall, 40);
        let l = compute(24, &w, 0, RESERVED_IDLE, RESERVED_IDLE);
        assert_eq!(l.container_height, 5, "上限应基于扣掉保留区后的高度");
    }

    #[test]
    fn inner_width_matches_border() {
        assert_eq!(inner_width(80), 76);
        assert_eq!(inner_width(5), 1);
        assert_eq!(inner_width(2), 1, "极窄终端不返回 0");
    }
}
