//! Truncation by display width.

use super::width::char_width;

/// Keep the longest prefix that fits; return it with its width.
pub fn take_width(s: &str, max: usize) -> (String, usize) {
    let mut out = String::new();
    let mut width = 0usize;
    for ch in s.chars() {
        let cell = char_width(ch);
        if width + cell > max {
            break;
        }
        out.push(ch);
        width += cell;
    }
    (out, width)
}
