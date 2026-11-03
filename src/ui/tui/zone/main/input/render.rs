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
use crate::ui::tui::theme::Accent;
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
pub fn render(spec: &Spec<'_>, accent: &Accent) -> InputView {
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
        lines.push(bottom_line("", w, accent));
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
        lines.push(bar_line(&wrapped.lines[i], w, accent));
    }
    lines.push(bottom_line(&wrapped.lines[bottom_row], w, accent));

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
fn bar_line(content: &str, w: usize, accent: &Accent) -> Line<'static> {
    let inner = geometry::inner_width(w as u16);
    let (seg, seg_w) = text::take_width(content, inner);
    let pad = inner.saturating_sub(seg_w);
    Line::from(vec![
        accent.span("| "),
        Span::styled(seg, Style::new()),
        // Padding is spaces: the user types here.
        Span::styled(" ".repeat(pad), Style::new()),
        accent.span(" |"),
    ])
}

/// The bottom edge: `+-{content}`, padding, `-+`. Both ends are one dash shorter
/// than the statusline.
fn bottom_line(content: &str, w: usize, accent: &Accent) -> Line<'static> {
    let inner = geometry::inner_width(w as u16);
    // Too wide is truncated by display width; the border never breaks.
    let (seg, seg_w) = text::take_width(content, inner);
    let pad = inner.saturating_sub(seg_w);
    Line::from(vec![
        accent.span("+-"),
        Span::styled(seg, Style::new()),
        Span::styled(" ".repeat(pad), Style::new()),
        accent.span("-+"),
    ])
}
