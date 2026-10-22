//! The session price: currency symbol and two decimals.

use ratatui::text::Span;

use crate::ui::tui::zone::Ctx;
use crate::ui::tui::zone::main::input::statusline::theme::{StatusTheme, Token};
use crate::ui::tui::zone::main::input::statusline::{Deco, Setup};

#[derive(Debug, Default)]
pub struct Cost;

pub(super) fn make(_: &Setup) -> Box<dyn Deco> {
    Box::new(Cost::default())
}

impl Deco for Cost {
    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        let Some(panel) = ctx.view.usage else {
            return Vec::new();
        };
        let mut total = 0.0;
        let mut symbol = "";
        for g in &panel.groups {
            let Some(cost) = &g.cost else { continue };
            total += cost.input + cost.output + cost.cache_read + cost.cache_write;
            if symbol.is_empty() {
                symbol = g.currency.as_deref().unwrap_or("");
            }
        }
        // No currency or a zero total prints nothing.
        if symbol.is_empty() || total <= 0.0 {
            return Vec::new();
        }
        vec![t.capsule(
            crate::ui::tui::zone::main::num::money(total, symbol),
            t.get(Token::Gold),
        )]
    }
}
