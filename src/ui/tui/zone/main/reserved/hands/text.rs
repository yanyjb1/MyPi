//! Cells: cut, fit and wrap a run of text to a column.

use crate::ui::tui::text::{display_width, take_width};

/// Cut to `w` cells, marking the cut when it does not fit.
pub(super) fn clip(text: &str, w: usize) -> String {
    if display_width(text) <= w {
        return text.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let (mut cut, _) = take_width(text, w.saturating_sub(1));
    cut.push('…');
    cut
}

/// Left-aligned in `w` cells.
pub(super) fn fit(text: &str, w: usize) -> String {
    let mut out = clip(text, w);
    let pad = w.saturating_sub(display_width(&out));
    out.push_str(&" ".repeat(pad));
    out
}

/// Right-aligned in `w` cells.
pub(super) fn right(text: &str, w: usize) -> String {
    let out = clip(text, w);
    let pad = w.saturating_sub(display_width(&out));
    format!("{}{out}", " ".repeat(pad))
}

/// One logical line, wrapped to `w` cells; an empty line stays one empty row.
pub(super) fn wrap(text: &str, w: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() && w > 0 {
        let (head, used) = take_width(rest, w);
        if used == 0 {
            out.push(rest.to_string());
            break;
        }
        rest = &rest[head.len()..];
        out.push(head);
    }
    out
}
