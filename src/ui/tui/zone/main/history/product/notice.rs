//! A notice row: severity picks the color, nothing else.

use crate::ui::tui::theme::fixed;
use crate::ui::tui::zone::main::history::local::slot::Level;
use crate::ui::tui::zone::main::history::row::{Ink, Lead, Row};

/// One notice, one row per line.
pub(crate) fn rows(text: &str, level: Level) -> Vec<Row> {
    let color = match level {
        Level::Info => fixed::SYSTEM,
        Level::Warn => fixed::WARN,
        Level::Error => fixed::ERROR_TEXT,
    };
    text.split('\n')
        .map(|part| Row::text(Lead::None, part.to_string(), Some(Ink::Fixed(color))))
        .collect()
}
