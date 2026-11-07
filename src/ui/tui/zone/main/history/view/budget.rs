//! The sight line's own arithmetic: 0 does not occupy a row.

/// How many rows sit above and below the sight line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Rows above it, numbered -1 backwards.
    pub above: usize,
    /// Rows below it, numbered 1 forwards.
    pub below: usize,
}

impl Budget {
    pub fn of(height: u16, focus_line: u16) -> Budget {
        let height = usize::from(height.max(1));
        let above = usize::from(focus_line).min(height.saturating_sub(1));
        Budget {
            above,
            below: height.saturating_sub(above),
        }
    }

    /// Rows the screen has, the sight line itself taking none.
    pub fn height(&self) -> usize {
        self.above + self.below
    }
}
