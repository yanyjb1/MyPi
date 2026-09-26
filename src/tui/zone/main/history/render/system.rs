//! System notices and session errors — the muted/red informational rows.
//! The emitter chooses alignment; this file only places and styles.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::server::entry::Align;
use crate::tui::text::display_width;
use super::theme::{HistoryTheme, Token};

// System notice: the emitter chose the alignment, we only place it.
pub(super) fn system_block(
    text: &str,
    align: Align,
    t: &HistoryTheme,
    width: usize,
) -> Vec<Line<'static>> {
    let style = Style::new().fg(t.get(Token::SystemText));
    let mut out = Vec::new();
    for part in text.split('\n') {
        let w = display_width(part);
        match align {
            Align::Left => out.push(Line::styled(part.to_string(), style)),
            Align::Center => {
                let pad = width.saturating_sub(w) / 2;
                out.push(Line::from(vec![
                    Span::styled(" ".repeat(pad), style),
                    Span::styled(part.to_string(), style),
                ]));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
