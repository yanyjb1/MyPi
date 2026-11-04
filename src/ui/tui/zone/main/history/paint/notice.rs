//! A notice row: severity picks the color, nothing else.

use super::super::layout::line::{Ink, Lead, Row};
use super::super::local::Level;
use crate::ui::tui::theme::fixed;

pub(crate) fn block(text: &str, level: Level) -> Vec<Row> {
    let color = match level {
        Level::Info => fixed::SYSTEM,
        Level::Warn => fixed::WARN,
        Level::Error => fixed::ERROR_TEXT,
    };
    text.split('\n')
        .map(|part| Row::text(Lead::None, part.to_string(), Some(Ink::Fixed(color))))
        .collect()
}
