//! The last command's answer, when this front end had to refuse it.

use ratatui::text::Span;

use super::super::theme::{Slot, StatusTheme};
use super::super::{Deco, Setup};
use crate::ui::tui::zone::main::input::Ctx;

#[derive(Debug, Default)]
pub struct Hint;

pub(super) fn make(_: &Setup) -> Box<dyn Deco> {
    Box::new(Hint::default())
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
