//! The session price — the model's currency symbol plus two decimals.
//!
//! Update source: `Usage`, emitted once per frame by the render loop from the
//! cost tracker (the same notification the usage gauge gets). The arithmetic
//! stays in `crate::server::ai::pricing`; this component only formats.
//!
//! Self-hides when the model has no price sheet (`show_cost == false`): there
//! is nothing meaningful to print, which is not the same as "the price is
//! zero".

use ratatui::text::Span;

use crate::tui::zone::main::input::statusline::theme::{ColorPolicy, StatusTheme, Token};
use crate::tui::zone::main::input::statusline::{PRIORITY_BASE, Side, StatusComponent, StatusEvent};

#[derive(Debug, Default)]
pub struct Cost {
    total: f64,
    symbol: &'static str,
    show: bool,
}

impl StatusComponent for Cost {
    fn id(&self) -> &'static str {
        "cost"
    }

    fn side(&self) -> Side {
        Side::Left
    }

    fn order(&self) -> u8 {
        40
    }

    fn priority(&self) -> u8 {
        PRIORITY_BASE
    }

    fn colors(&self) -> ColorPolicy {
        ColorPolicy::Tokens(&[Token::Gold, Token::Capsule])
    }

    fn on_event(&mut self, ev: &StatusEvent<'_>) {
        if let StatusEvent::Usage(u) = ev {
            self.total = u.total_cost;
            self.symbol = u.currency_symbol;
            self.show = u.show_cost;
        }
    }

    fn render(&self, t: &StatusTheme, _budget: Option<usize>) -> Vec<Span<'static>> {
        if !self.show || self.symbol.is_empty() {
            return Vec::new();
        }
        vec![t.capsule(fmt_money(self.total, self.symbol), t.get(Token::Gold))]
    }
}

/// Money: fixed two decimals, with the currency symbol.
fn fmt_money(v: f64, symbol: &str) -> String {
    format!("{symbol}{v:.2}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_is_two_decimals_with_the_symbol() {
        assert_eq!(fmt_money(0.0004, "$"), "$0.00");
        assert_eq!(fmt_money(2.77, "$"), "$2.77");
        assert_eq!(fmt_money(0.001, "¥"), "¥0.00");
        assert_eq!(fmt_money(1.5, "¥"), "¥1.50");
    }
}
