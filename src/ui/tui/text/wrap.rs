//! Wrapping by display width, with char-index mapping for the cursor.

use super::width::{char_width, display_width};

/// Wrapped rows and the flat char index where each row starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wrapped {
    pub lines: Vec<String>,
    pub starts: Vec<usize>,
}

impl Wrapped {
    /// Number of visual rows, at least one.
    pub fn len(&self) -> usize {
        self.lines.len().max(1)
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Flat char index to (row, cell offset).
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

    /// (row, cell offset) to flat char index.
    pub fn char_at(&self, row: usize, col: usize) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let row = row.min(self.lines.len() - 1);
        let line = &self.lines[row];
        let mut width = 0usize;
        let mut count = 0usize;
        for ch in line.chars() {
            let cell = char_width(ch);
            if width + cell > col {
                break;
            }
            width += cell;
            count += 1;
        }
        self.starts[row] + count
    }
}

/// Wrap text at `inner_w` cells; newlines are hard cuts.
pub fn wrap(text: &str, inner_w: usize) -> Wrapped {
    let inner_w = inner_w.max(1);
    let flat: Vec<char> = text.chars().collect();
    let mut lines = Vec::new();
    let mut starts = Vec::new();
    let mut cur = String::new();
    let mut width = 0usize;
    let mut start = 0usize;

    for (i, &ch) in flat.iter().enumerate() {
        if ch == '\n' {
            lines.push(std::mem::take(&mut cur));
            starts.push(start);
            width = 0;
            start = i + 1;
            continue;
        }
        let cell = char_width(ch);
        if width + cell > inner_w {
            lines.push(std::mem::take(&mut cur));
            starts.push(start);
            width = 0;
            start = i;
        }
        cur.push(ch);
        width += cell;
    }
    lines.push(cur);
    starts.push(start);
    Wrapped { lines, starts }
}
