//! The session name — `< name` at the right end of the row.
//!
//! Update source: `SessionRenamed`. The sender resolves the name (an explicit
//! `/name`, or the synthesized first-user-message fallback) because a component
//! cannot see the transcript — the row's data flow is strictly inward.

use ratatui::text::Span;

use crate::tui::zone::main::input::statusline::theme::{ColorPolicy, StatusTheme, Token};
use crate::tui::zone::main::input::statusline::{Side, StatusComponent, StatusEvent};

#[derive(Debug, Default)]
pub struct Session {
    name: String,
}

impl StatusComponent for Session {
    fn id(&self) -> &'static str {
        "session"
    }

    fn side(&self) -> Side {
        Side::Right
    }

    fn order(&self) -> u8 {
        0
    }

    fn priority(&self) -> u8 {
        60
    }

    fn colors(&self) -> ColorPolicy {
        ColorPolicy::Tokens(&[Token::Accent, Token::Capsule])
    }

    fn on_event(&mut self, ev: &StatusEvent<'_>) {
        if let StatusEvent::SessionRenamed(name) = ev {
            self.name = (*name).to_string();
        }
    }

    fn render(&self, t: &StatusTheme, _budget: Option<usize>) -> Vec<Span<'static>> {
        if self.name.is_empty() {
            return Vec::new();
        }
        vec![t.capsule(self.name.clone(), t.get(Token::Accent))]
    }
}
