//! The screen rows: laid rows with their color slots resolved.

use ratatui::text::Line;

use super::row::Row;
use crate::ui::tui::theme::Accent;

/// The screen's rows, in order.
pub fn lines(rows: &[Row], accent: Accent) -> Vec<Line<'static>> {
    rows.iter().map(|row| row.paint(accent)).collect()
}
