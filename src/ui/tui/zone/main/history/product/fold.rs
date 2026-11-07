//! The second layer: one natural line to the row breaks the terminal has room for.
//!
//! Only this layer knows a width, so a resize rebuilds it and nothing else.

use crate::ui::tui::text::display_width;

/// The char indices a natural line starts a new row at, given the columns a row leaves.
pub fn breaks(text: &str, avail: usize) -> Vec<usize> {
    let avail = avail.max(1);
    let mut out: Vec<usize> = Vec::new();
    let mut used = 0usize;
    for (at, ch) in text.chars().enumerate() {
        let width = display_width(&ch.to_string());
        if used + width > avail && used > 0 {
            out.push(at);
            used = 0;
        }
        used += width;
    }
    out
}
