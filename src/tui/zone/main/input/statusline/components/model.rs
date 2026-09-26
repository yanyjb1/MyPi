//! The model name — `[M] <display name>`.
//!
//! Update source: `ModelChanged`, emitted by `/switch` (the only thing that
//! changes the session's model). Passive by construction: no polling, no
//! lookup of the config from here.

use ratatui::text::Span;

use crate::tui::zone::main::input::statusline::theme::{ColorPolicy, StatusTheme, Token};
use crate::tui::zone::main::input::statusline::{Side, StatusComponent, StatusEvent};

#[derive(Debug, Default)]
pub struct Model {
    name: String,
}

impl StatusComponent for Model {
    fn id(&self) -> &'static str {
        "model"
    }

    fn side(&self) -> Side {
        Side::Left
    }

    fn order(&self) -> u8 {
        10
    }

    fn priority(&self) -> u8 {
        80
    }

    fn colors(&self) -> ColorPolicy {
        ColorPolicy::Tokens(&[Token::Accent, Token::Capsule])
    }

    fn on_event(&mut self, ev: &StatusEvent<'_>) {
        if let StatusEvent::ModelChanged(name) = ev {
            self.name = (*name).to_string();
        }
    }

    fn render(&self, t: &StatusTheme, _budget: Option<usize>) -> Vec<Span<'static>> {
        if self.name.is_empty() {
            return Vec::new(); // not told yet: nothing to say
        }
        vec![t.capsule(
            format!("[M] {}", self.name),
            t.get(Token::Accent),
        )]
    }
}
