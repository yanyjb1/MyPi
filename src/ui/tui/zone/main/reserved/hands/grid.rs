//! A table row: cells cut and padded to their columns, the cursor's row accented.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::tui::theme::{Accent, fixed};

use super::text;

/// The block's columns: width, alignment, the gap between them, its left margin.
pub struct Grid<'a> {
    pub widths: &'a [usize],
    pub right: &'a [bool],
    pub gap: usize,
    pub lead: usize,
}

impl Grid<'_> {
    /// The head row, MUTED.
    pub fn head(&self, cells: &[String]) -> Line<'static> {
        Line::from(Span::styled(
            self.line(cells),
            Style::new().fg(fixed::MUTED),
        ))
    }

    /// One data row, accented when the cursor sits on it.
    pub fn row(&self, cells: &[String], selected: bool, accent: &Accent) -> Line<'static> {
        let text = self.line(cells);
        match selected {
            true => Line::from(accent.span(text)),
            false => Line::from(Span::raw(text)),
        }
    }

    fn line(&self, cells: &[String]) -> String {
        let mut out = " ".repeat(self.lead);
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                out.push_str(&" ".repeat(self.gap));
            }
            let width = self.widths.get(i).copied().unwrap_or(0);
            let cell = match self.right.get(i).copied().unwrap_or(false) {
                true => text::right(cell, width),
                false => text::fit(cell, width),
            };
            out.push_str(&cell);
        }
        out
    }
}
