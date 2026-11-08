//! The last command's answer, when this front end had to refuse it.

use ratatui::text::Span;

use super::super::Deco;
use super::super::theme::{Slot, StatusTheme};
use crate::ui::tui::zone::main::input::Ctx;

#[derive(Debug)]
pub struct Hint;

pub(super) fn make() -> Box<dyn Deco> {
    Box::new(Hint)
}

impl Deco for Hint {
    fn render(&self, t: &StatusTheme, budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        let Some(text) = ctx.data.hint.as_deref() else {
            return Vec::new();
        };
        match budget {
            Some(width) if crate::ui::tui::text::display_width(text) > width => Vec::new(),
            _ => vec![t.fg(text, Slot::Warn)],
        }
    }
}
