//! Input zone rendering: the statusline is the top edge, body rows wear `| .. |`,
//! the last visible row runs into the bottom edge.
//!
//! ```text
//! +--pi > [M] model > [D] path ... --+     <- statusline, 1 row
//! | first body line                  |     <- only when the body is longer
//! | second body line                 |
//! +-last body line-------------------+     <- bottom edge, `+-` / `-+`
//! ```
//!
//! Rules: the statusline is exactly the container's width; the bottom edge holds
//! content only; wrapping is by display width. Geometry lives in
//! [`crate::ui::tui::zone::main::geometry`].

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::tui::text;
use crate::ui::tui::theme::Palette;
use crate::ui::tui::zone::main::geometry;

/// What the renderer produced: the rows, and where the cursor sits in them.
#[derive(Debug, Clone)]
pub struct InputView {
    /// Every container row; row 0 is the statusline.
    pub lines: Vec<Line<'static>>,
    /// Cursor row, 0 = the statusline row.
    pub cursor_row: usize,
    /// Cursor column in terminal cells.
    pub cursor_col: usize,
}

/// Everything the renderer reads.
pub struct Spec<'a> {
    /// The statusline row, used as the top edge.
    pub status_line: Line<'static>,
    /// The input text, wrapped to `inner_width`.
    pub wrapped: &'a text::Wrapped,
    /// First visible wrapped row.
    pub starts: usize,
    /// Container rows, statusline and bottom edge included.
    pub container_rows: usize,
    /// The cursor's wrapped row, absolute.
    pub cursor_row: usize,
    /// The cursor's cell offset inside that wrapped row.
    pub cursor_col: usize,
    /// Terminal width.
    pub term_w: u16,
}

/// Draw the input zone.
pub fn render(spec: &Spec<'_>, p: &Palette) -> InputView {
    let w = spec.term_w as usize;
    // The statusline is exactly the container's width; one cell off tears the
    // rows below, so both sides are held to it here.
    debug_assert_eq!(
        spec.status_line
            .spans
            .iter()
            .map(|s| text::display_width(&s.content))
            .sum::<usize>(),
        w,
        "statusline must be exactly {w} cells"
    );

    let wrapped = spec.wrapped;
    let n = wrapped.len();
    let starts = spec.starts.min(n.saturating_sub(1));
    // `| .. |` rows = container rows - statusline - bottom edge.
    let body_rows = spec.container_rows.saturating_sub(2);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(spec.container_rows);
    lines.push(spec.status_line.clone());

    if n == 0 {
        // An empty input still gets a bottom edge to stand on.
        lines.push(bottom_line("", w, p));
        return InputView {
            lines,
            cursor_row: 1,
            cursor_col: 2,
        };
    }

    // The viewport covers `body_rows + 1` rows: the first `body_rows` wear
    // `| .. |`, the last one runs into the bottom edge.
    let visible = body_rows + 1;
    let bottom_row = (starts + visible - 1).min(n - 1);
    for i in starts..bottom_row {
        lines.push(bar_line(&wrapped.lines[i], w, p));
    }
    lines.push(bottom_line(&wrapped.lines[bottom_row], w, p));

    // The statusline takes the first row, scrolled rows come off the top, and both
    // body rows and the bottom edge carry a two-cell prefix.
    let row = 1 + spec.cursor_row.saturating_sub(starts);
    let col = 2 + spec.cursor_col;
    InputView {
        lines,
        cursor_row: row,
        cursor_col: col,
    }
}

/// A body row: `| content |`, left aligned, padded to the full width.
fn bar_line(content: &str, w: usize, p: &Palette) -> Line<'static> {
    let inner = geometry::inner_width(w as u16);
    let (seg, seg_w) = text::take_width(content, inner);
    let pad = inner.saturating_sub(seg_w);
    Line::from(vec![
        p.accent_span("| "),
        Span::styled(seg, Style::new()),
        // Padding is spaces: the user types here.
        Span::styled(" ".repeat(pad), Style::new()),
        p.accent_span(" |"),
    ])
}

/// The bottom edge: `+-{content}`, padding, `-+`. Both ends are one dash shorter
/// than the statusline.
fn bottom_line(content: &str, w: usize, p: &Palette) -> Line<'static> {
    let inner = geometry::inner_width(w as u16);
    // Too wide is truncated by display width; the border never breaks.
    let (seg, seg_w) = text::take_width(content, inner);
    let pad = inner.saturating_sub(seg_w);
    Line::from(vec![
        p.accent_span("+-"),
        Span::styled(seg, Style::new()),
        Span::styled(" ".repeat(pad), Style::new()),
        p.accent_span("-+"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    fn width_of(l: &Line) -> usize {
        l.spans.iter().map(|s| s.content.width()).sum()
    }

    fn text_of(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.to_string()).collect()
    }

    fn status(w: u16) -> Line<'static> {
        Line::from("+".to_string() + &"-".repeat(w as usize - 2) + "+")
    }

    fn spec<'a>(
        wrapped: &'a text::Wrapped,
        container_rows: usize,
        starts: usize,
        cursor_row: usize,
        cursor_col: usize,
        term_w: u16,
    ) -> Spec<'a> {
        Spec {
            status_line: status(term_w),
            wrapped,
            starts,
            container_rows,
            cursor_row,
            cursor_col,
            term_w,
        }
    }

    /// A one-line input: statusline plus bottom edge, the edge carrying the text.
    #[test]
    fn single_line_input_is_status_then_bottom() {
        let p = Palette::default();
        let wrapped = text::wrap("hi", 40);
        let v = render(&spec(&wrapped, 2, 0, 0, 2, 20), &p);
        assert_eq!(v.lines.len(), 2, "statusline plus bottom edge");
        assert!(
            text_of(&v.lines[1]).starts_with("+-hi"),
            "{:?}",
            text_of(&v.lines[1])
        );
        assert!(text_of(&v.lines[1]).ends_with("-+"));
    }

    /// A multi-line input: middle rows wear `| .. |`, the last one takes the edge.
    #[test]
    fn multiline_input_gets_bar_rows() {
        let p = Palette::default();
        let wrapped = text::wrap("aa\nbb\ncc", 40);
        let v = render(&spec(&wrapped, 4, 0, 2, 2, 20), &p);
        assert_eq!(v.lines.len(), 4, "statusline plus two bars plus edge");
        assert!(text_of(&v.lines[1]).starts_with("| aa"));
        assert!(text_of(&v.lines[2]).starts_with("| bb"));
        assert!(text_of(&v.lines[3]).starts_with("+-cc"));
    }

    /// Every row is exactly the terminal's width.
    #[test]
    fn every_row_is_exactly_terminal_wide() {
        let p = Palette::default();
        for term_w in [8u16, 20, 40, 81] {
            let wrapped = text::wrap("一段中文 abcd efgh\n第二行", geometry::inner_width(term_w));
            let v = render(&spec(&wrapped, 5, 0, 1, 1, term_w), &p);
            for (i, l) in v.lines.iter().enumerate() {
                assert_eq!(
                    width_of(l),
                    term_w as usize,
                    "({term_w}) row {i} has the wrong width: {:?}",
                    text_of(l)
                );
            }
        }
    }

    /// The bottom edge follows the viewport, not the end of the text.
    #[test]
    fn bottom_edge_follows_the_viewport_not_the_text_end() {
        let p = Palette::default();
        let wrapped = text::wrap("a\nb\nc\nd\ne", 40);
        // Three container rows: statusline, one bar, edge; the viewport covers two.
        let v = render(&spec(&wrapped, 3, 0, 0, 0, 20), &p);
        assert!(text_of(&v.lines[1]).starts_with("| a"));
        assert!(text_of(&v.lines[2]).starts_with("+-b"), "edge shows row 2");
        // Scrolled to row 3.
        let v = render(&spec(&wrapped, 3, 2, 2, 0, 20), &p);
        assert!(text_of(&v.lines[1]).starts_with("| c"));
        assert!(
            text_of(&v.lines[2]).starts_with("+-d"),
            "edge did not follow"
        );
    }

    /// An empty input keeps a bottom edge, and the cursor sits on it.
    #[test]
    fn empty_input_still_has_a_bottom_edge() {
        let p = Palette::default();
        let wrapped = text::wrap("", 40);
        let v = render(&spec(&wrapped, 2, 0, 0, 0, 20), &p);
        assert_eq!(v.lines.len(), 2);
        assert_eq!(text_of(&v.lines[1]), format!("+-{}-+", " ".repeat(16)));
        assert_eq!((v.cursor_row, v.cursor_col), (1, 2));
    }

    /// The cursor row tracks the viewport and stays inside the container.
    #[test]
    fn cursor_row_tracks_the_viewport() {
        let p = Palette::default();
        let wrapped = text::wrap("a\nb\nc\nd", 40);
        let v = render(&spec(&wrapped, 4, 2, 3, 0, 20), &p);
        assert_eq!(v.cursor_row, 1 + 3 - 2, "cursor row did not follow");
        assert!(v.cursor_row < v.lines.len(), "cursor left the container");
    }
}
