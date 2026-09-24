//! Input container rendering — statusline as the top edge, `|` side rows, `+-{last row}...+` as the bottom edge.
//!
//! Layout (user spec):
//! ```text
//! +--pi > [M]model > [D]path ... --+     <- statusline (1 row, top edge)
//! | first input row                  |    <- appears only when input > 1 row
//! | second input row                 |
//! +-last input row------------------+    <- bottom edge; +- / -+ ends, blanks between
//! ```
//!
//! Key points:
//! - the statusline ends `+--` / `--+`; the bottom edge is `+-` / `-+`, one dash shorter;
//! - the bottom edge middle holds **only content, blank otherwise** (never dash-filled): the user types there;
//! - wrapping uses terminal display width (see `text.rs`).

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::tui::theme::Palette;

// The input area's render result: row list + cursor coordinates inside the container.
#[derive(Debug, Clone)]
pub struct InputView {
    // All container rows as `Line`s (row 0 is the statusline, supplied by the caller).
    pub lines: Vec<Line<'static>>,
    // Cursor row offset from the container top (0 = the statusline row).
    pub cursor_row: usize,
    // Cursor column offset from the terminal left (cells).
    pub cursor_col: usize,
}

// Everything needed to render the input area.
//
// Collected into a struct rather than 8 positional parameters:
// `starts` / `visible_rows` / `cursor_row` / `cursor_col` are all `usize`;
// swapped positions compile silently and render crooked.
pub struct InputSpec<'a> {
    // The statusline row (from `statusline::render`), serving as the container top edge.
    pub status_line: Line<'static>,
    // The input text wrapped at `inner_width`.
    pub wrapped: &'a crate::tui::text::Wrapped,
    // First wrapped row shown by the viewport (see `layout`).
    pub starts: usize,
    // Wrapped rows the viewport covers (`|` rows + bottom-edge row = container height - 1).
    pub visible_rows: usize,
    // Cursor row within the wrapped result (absolute, not viewport-relative).
    pub cursor_row: usize,
    // Cursor cell offset within that visual row.
    pub cursor_col: usize,
    // Terminal width.
    pub term_w: u16,
}

// Render the input area.
pub fn render(spec: &InputSpec<'_>, p: &Palette) -> InputView {
    let w = spec.term_w as usize;
    let wrapped = spec.wrapped;
    let starts = spec.starts;
    let n = wrapped.len();
    let last_idx = n.saturating_sub(1);

    // Viewport: wrapped rows [starts, ends); the last row goes to the bottom edge.
    // The bottom edge draws the viewport's last row, not the text's last row —
    // otherwise, after scrolling up, the edge stays pinned to the text end and the picture tears.
    let ends = (starts + spec.visible_rows.max(1)).min(n);
    let bottom_row = ends.saturating_sub(1).min(last_idx);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(spec.status_line.clone()); // 第 0 行：状态栏
    for i in starts..bottom_row {
        lines.push(bar_line(&wrapped.lines[i], w, p));
    }
    let last = wrapped
        .lines
        .get(bottom_row)
        .map(|s| s.as_str())
        .unwrap_or("");
    lines.push(bottom_line(last, w, p));

    // Cursor row: the statusline takes 1 row, minus the rows before the viewport start
    let row = 1 + spec.cursor_row.saturating_sub(starts);
    // Content rows and the bottom edge both carry a 2-cell prefix (`| ` / `+-`)
    let col = 2 + spec.cursor_col;

    InputView {
        lines,
        cursor_row: row,
        cursor_col: col,
    }
}

// A content row: `| {content} |` with accent bars on both sides, flush left, padded to full width.
fn bar_line(content: &str, w: usize, p: &Palette) -> Line<'static> {
    // The border width has a single definition in `layout::BORDER_COLS`; never re-write 4 here
    let inner = crate::tui::layout::inner_width(w as u16);
    let cw = crate::tui::text::display_width(content);
    let pad = inner.saturating_sub(cw);
    Line::from(vec![
        p.accent_span("| "),
        Span::styled(content.to_string(), Style::new()),
        p.accent_span(format!("{} |", " ".repeat(pad))),
    ])
}

// Bottom edge: `+-{content}` + blanks + `-+`. One dash shorter than the statusline at each end.
fn bottom_line(content: &str, w: usize, p: &Palette) -> Line<'static> {
    let inner = crate::tui::layout::inner_width(w as u16);
    // Truncate by display width when too wide, so the border never breaks
    let (seg, seg_w) = crate::tui::text::take_width(content, inner);
    let pad = inner.saturating_sub(seg_w);
    Line::from(vec![
        p.accent_span("+-"),
        Span::styled(seg, Style::new()),
        p.plain(" ".repeat(pad)), // 留空，用户在此继续输入
        p.accent_span("-+"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::text;
    use unicode_width::UnicodeWidthStr;

    fn width_of(l: &Line) -> usize {
        l.spans.iter().map(|s| s.content.width()).sum()
    }

    fn status() -> Line<'static> {
        Line::from("+--status--+")
    }

    // Assemble an InputSpec for tests (most cases only care about starts/cursor/width).
    fn spec(
        wrapped: &crate::tui::text::Wrapped,
        starts: usize,
        cursor_row: usize,
        cursor_col: usize,
        term_w: u16,
    ) -> InputSpec<'_> {
        InputSpec {
            status_line: status(),
            wrapped,
            starts,
            visible_rows: 99,
            cursor_row,
            cursor_col,
            term_w,
        }
    }

    #[test]
    fn single_line_input_has_status_then_bottom() {
        let p = Palette::default();
        let wrapped = text::wrap("hi", 40);
        let v = render(&spec(&wrapped, 0, 0, 2, 20), &p);
        assert_eq!(v.lines.len(), 2, "单行输入：状态栏 + 底边");
        let text_of =
            |l: &Line| -> String { l.spans.iter().map(|s| s.content.to_string()).collect() };
        assert!(text_of(&v.lines[1]).starts_with("+-hi"));
        assert!(text_of(&v.lines[1]).ends_with("-+"));
    }

    #[test]
    fn multiline_input_gets_bar_rows() {
        let p = Palette::default();
        let wrapped = text::wrap("aa\nbb\ncc", 40);
        let v = render(&spec(&wrapped, 0, 2, 2, 20), &p);
        assert_eq!(v.lines.len(), 4); // 状态栏 + 2 个 | 行 + 底边
        let text_of =
            |l: &Line| -> String { l.spans.iter().map(|s| s.content.to_string()).collect() };
        assert!(text_of(&v.lines[1]).starts_with("| aa"));
        assert!(text_of(&v.lines[2]).starts_with("| bb"));
        assert!(text_of(&v.lines[3]).starts_with("+-cc"));
    }

    #[test]
    fn all_rows_fill_terminal_width() {
        let p = Palette::default();
        let wrapped = text::wrap("aa\nbb\ncc", 40);
        let v = render(&spec(&wrapped, 0, 0, 0, 40), &p);
        for (i, l) in v.lines.iter().enumerate().skip(1) {
            // The statusline is caller-supplied and unchecked; content rows must fill the width
            assert_eq!(width_of(l), 40, "row {i}");
        }
    }

    #[test]
    fn bottom_line_leaves_space_instead_of_dashes() {
        let p = Palette::default();
        let wrapped = text::wrap("x", 40);
        let v = render(&spec(&wrapped, 0, 0, 1, 20), &p);
        let t: String = v.lines[1]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        // Blanks in the middle, not dashes
        assert!(t.contains("+-x "), "{t:?}");
        assert!(!t.contains("---"), "底边不该用 - 填满: {t:?}");
    }

    #[test]
    fn cursor_row_offset_accounts_for_status_line() {
        let p = Palette::default();
        let wrapped = text::wrap("aa\nbb\ncc", 40);
        // Cursor on the first content visual row ("bb") -> container row 2 (0=statusline, 1=aa, 2=bb)
        let v = render(&spec(&wrapped, 0, 1, 0, 40), &p);
        assert_eq!(v.cursor_row, 2);
        assert_eq!(v.cursor_col, 2);
    }

    #[test]
    fn cursor_col_includes_prompt_prefix() {
        let p = Palette::default();
        let wrapped = text::wrap("中文", 40);
        let v = render(&spec(&wrapped, 0, 0, 0, 40), &p);
        assert_eq!(v.cursor_col, 2, "`+-` 占 2 格");
        // Cursor after two full-width chars -> 2 + 4 = 6
        let v2 = render(&spec(&wrapped, 0, 0, 4, 40), &p);
        assert_eq!(v2.cursor_col, 6);
    }

    #[test]
    fn long_content_is_truncated_not_wrapped() {
        let p = Palette::default();
        let long = "x".repeat(100);
        let wrapped = text::wrap(&long, 10); // 折成多行
        let v = render(&spec(&wrapped, 0, 0, 0, 20), &p);
        // Every row within the terminal width
        for (i, l) in v.lines.iter().enumerate().skip(1) {
            assert_eq!(width_of(l), 20, "row {i}");
        }
    }

    #[test]
    fn cjk_content_width_is_cells() {
        let p = Palette::default();
        let wrapped = text::wrap("中文", 40);
        let v = render(&spec(&wrapped, 0, 0, 0, 20), &p);
        // "+-" + CJK text (4 cells) + blanks + "-+"
        let t: String = v.lines[1]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert_eq!(width_of(&v.lines[1]), 20);
        assert!(t.starts_with("+-中文"), "{t:?}");
    }
}
