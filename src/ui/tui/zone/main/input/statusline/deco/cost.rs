//! The session price: the model's own currency, two decimals.

use ratatui::text::Span;

use super::super::theme::{StatusTheme, Token};
use super::super::{Deco, Setup};
use crate::ui::tui::zone::main::input::Ctx;

#[derive(Debug, Default)]
pub struct Cost;

pub(super) fn make(_: &Setup) -> Box<dyn Deco> {
    Box::new(Cost::default())
}

impl Deco for Cost {
    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        let Some(text) = ctx.data.money.text() else {
            return Vec::new();
        };
        vec![t.capsule(text, t.get(Token::Gold))]
    }
}
