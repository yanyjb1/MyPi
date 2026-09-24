//! Text measurement and wrapping — everything is based on **display width**, not char counts or byte counts.
//!
//! Why `len()` will not do:
//! - `String::len()` counts bytes: a CJK char is 3 bytes but occupies 2 cells;
//! - `chars().count()` counts chars: a CJK char is 1 but occupies 2 cells, misaligning it with ASCII.
//!
//! The terminal lays text out by cells, so every width computation goes through `UnicodeWidthStr`.
//! Full-width CJK takes 2 cells, emoji usually 2, ASCII 1 — only cell-based math lines up
//! when they are mixed.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// How many cells a single character occupies.
pub fn char_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

/// How many cells a piece of text occupies.
pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Truncate by display width; returns (the kept text, its display width).
pub fn take_width(s: &str, max: usize) -> (String, usize) {
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = char_width(ch);
        if w + cw > max {
            break;
        }
        out.push(ch);
        w += cw;
    }
    (out, w)
}

/// Result of wrapping a piece of text.
///
/// `lines`: the visual rows, in order; one logical line may span several visual rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wrapped {
    pub lines: Vec<String>,
    /// For each row, the index of its first character in the flat char sequence; maps character positions to visual rows.
    pub starts: Vec<usize>,
}

impl Wrapped {
    /// Number of visual rows (at least 1).
    pub fn len(&self) -> usize {
        self.lines.len().max(1)
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Given a flat char index, returns (visual row index, cell offset within that row).
    ///
    /// Logic: find the last row with `starts[i] <= char_idx`; the offset is the display
    /// width of that row's `[starts[i], char_idx)` slice.
    pub fn locate(&self, char_idx: usize) -> (usize, usize) {
        if self.lines.is_empty() {
            return (0, 0);
        }
        let mut row = 0usize;
        for (i, &start) in self.starts.iter().enumerate() {
            if start <= char_idx {
                row = i;
            } else {
                break;
            }
        }
        let from = self.starts[row];
        let to = char_idx.min(from + self.lines[row].chars().count());
        let col = display_width(&self.lines[row].chars().take(to - from).collect::<String>());
        (row, col)
    }

    /// Given (visual row, target cell offset), returns the flat char index at that position.
    /// Used to keep the column while moving up/down.
    pub fn char_at(&self, row: usize, col: usize) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let row = row.min(self.lines.len() - 1);
        let line = &self.lines[row];
        let mut w = 0usize;
        let mut n = 0usize;
        for ch in line.chars() {
            let cw = char_width(ch);
            if w + cw > col {
                break;
            }
            w += cw;
            n += 1;
        }
        self.starts[row] + n
    }
}

/// Wrap whole text (may contain `\n`) to the display width `inner_w`.
///
/// For every row this also records the index of its first character in the **flat char
/// sequence** (`\n` counts as a character) — that mapping is what makes the
/// cursor ↔ visual-row correspondence possible.
pub fn wrap(text: &str, inner_w: usize) -> Wrapped {
    let inner_w = inner_w.max(1);
    let flat: Vec<char> = text.chars().collect();

    let mut lines: Vec<String> = Vec::new();
    let mut starts: Vec<usize> = Vec::new();

    let mut cur = String::new();
    let mut cur_w = 0usize;
    let mut cur_start = 0usize;

    for (i, &ch) in flat.iter().enumerate() {
        if ch == '\n' {
            lines.push(std::mem::take(&mut cur));
            starts.push(cur_start);
            cur_w = 0;
            cur_start = i + 1;
            continue;
        }
        let cw = char_width(ch);
        if cur_w + cw > inner_w {
            lines.push(std::mem::take(&mut cur));
            starts.push(cur_start);
            cur_w = 0;
            cur_start = i;
        }
        cur.push(ch);
        cur_w += cw;
    }
    lines.push(cur);
    starts.push(cur_start);

    Wrapped { lines, starts }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_count_cells_not_chars() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("中文"), 4); // not 2 (chars) nor 6 (bytes)
        assert_eq!(display_width("a中"), 3);
    }

    #[test]
    fn take_width_respects_cells() {
        let (s, w) = take_width("a中b", 3);
        assert_eq!(s, "a中");
        assert_eq!(w, 3);
        // Must not cut in the middle of a full-width character
        let (s2, w2) = take_width("中中", 3);
        assert_eq!(s2, "中");
        assert_eq!(w2, 2);
    }

    #[test]
    fn wrap_splits_on_newline_and_width() {
        // Width 4, 2 cells per full-width char → 2 CJK chars per row
        let w = wrap("中文字", 4);
        assert_eq!(w.lines, vec!["中文", "字"]);
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn wrap_keeps_empty_logical_lines() {
        let w = wrap("a\n\nb", 10);
        assert_eq!(w.lines, vec!["a", "", "b"]);
    }

    #[test]
    fn wrap_empty_is_one_line() {
        let w = wrap("", 10);
        assert_eq!(w.lines, vec![""]);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn locate_maps_char_index_to_row_col() {
        // CJK example: a 6-cell text at width 4 wraps to two rows
        // starts = [0, 2], char indices 0..=3
        let w = wrap("中文字", 4);
        assert_eq!(w.starts, vec![0, 2]);
        assert_eq!(w.locate(0), (0, 0)); // cursor before "中"
        assert_eq!(w.locate(1), (0, 2)); // before "文" → row 0, cell 2
        assert_eq!(w.locate(2), (1, 0)); // before "字" → row 1, cell 0
        assert_eq!(w.locate(3), (1, 2)); // end of text
    }

    #[test]
    fn locate_handles_newlines() {
        // "ab\ncd" → ["ab","cd"], starts=[0,3] (the \n occupies one char slot)
        let w = wrap("ab\ncd", 10);
        assert_eq!(w.starts, vec![0, 3]);
        assert_eq!(w.locate(2), (0, 2)); // before the \n
        assert_eq!(w.locate(3), (1, 0)); // after the \n = start of row 1
    }

    #[test]
    fn char_at_is_inverse_of_locate() {
        let w = wrap("中文字", 4);
        for idx in 0..=3 {
            let (row, col) = w.locate(idx);
            assert_eq!(w.char_at(row, col), idx, "idx={idx}");
        }
    }

    #[test]
    fn char_at_clamps_to_line_end() {
        let w = wrap("中文\nx", 4);
        // Row 0 has only 2 chars; asking for a cell beyond the end falls back to the row end
        assert_eq!(w.char_at(0, 99), 2);
    }
}
