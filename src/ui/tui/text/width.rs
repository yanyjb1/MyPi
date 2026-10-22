//! Display-cell width. Terminal layout counts cells, not chars or bytes.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Cells one character occupies.
pub fn char_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Cells a piece of text occupies.
pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}
