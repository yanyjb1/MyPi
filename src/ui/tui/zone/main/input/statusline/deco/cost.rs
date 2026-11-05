//! The session price: the model's own currency, two decimals.

use ratatui::text::Span;

use super::super::theme::{Slot, StatusTheme};
use super::super::Deco;
use crate::ui::tui::zone::main::input::Ctx;

#[derive(Debug, Default)]
pub struct Cost;

pub(super) fn make() -> Box<dyn Deco> {
    Box::new(Cost::default())
}

impl Deco for Cost {
    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        // The price pill never hides: zero is a fact, and a missing one reads as zero.
        vec![t.capsule(ctx.data.money.text(), t.get(Slot::Gold))]
    }
}
