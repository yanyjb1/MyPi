//! A notice row: severity picks the color, nothing else.

use ratatui::style::Style;
use ratatui::text::Line;

use crate::ui::session::Level;
use crate::ui::tui::theme::{HistoryTheme, Token};

pub(crate) fn notice_block(text: &str, level: Level, t: &HistoryTheme) -> Vec<Line<'static>> {
    let style = Style::new().fg(t.get(match level {
        Level::Info => Token::SystemText,
        Level::Error => Token::ErrorText,
    }));
    text.split('\n')
        .map(|part| Line::styled(part.to_string(), style))
        .collect()
}
