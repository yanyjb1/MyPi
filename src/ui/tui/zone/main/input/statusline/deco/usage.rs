//! The usage gauge — `>----8%----:1M`. The row's flexible deco.

use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use crate::ui::tui::zone::Ctx;
use crate::ui::tui::zone::main::input::statusline::theme::{StatusTheme, Token};
use crate::ui::tui::zone::main::input::statusline::{Deco, Setup};

#[derive(Debug, Default)]
pub struct Usage;

pub(super) fn make(_: &Setup) -> Box<dyn Deco> {
    Box::new(Usage::default())
}

impl Deco for Usage {
    fn render(&self, t: &StatusTheme, budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        let Some(limit) = ctx.view.ctx_limit.filter(|n| *n > 0) else {
            return Vec::new();
        };
        let now = ctx.view.ctx_tokens.unwrap_or(0);
        let pct = ((now.min(limit) as f64 / limit as f64 * 100.0).round() as u64).min(100);
        let pct_txt = format!("{pct}%");
        let denom = format!(":{}", fmt_tokens(limit));
        // Cells outside the overhead belong to the dashes.
        let overhead = 1 + pct_txt.width() + denom.width();
        let cells = budget.unwrap_or(overhead + 8).max(overhead + 1);
        let dash_w = cells - overhead;
        // The percentage sits between the dashes: accent counts the used share,
        // plain the rest. At 0% the first accent cell stays.
        let used = (pct as usize * dash_w / 100).min(dash_w);
        let used = if dash_w > 0 { used.max(1) } else { 0 };
        let plain_rest = dash_w - used;

        let mut out = vec![t.fg(">", Token::Accent)];
        if used > 0 {
            out.push(t.fg("-".repeat(used), Token::Accent));
        }
        out.push(t.fg(pct_txt, Token::Accent));
        if plain_rest > 0 {
            out.push(t.plain("-".repeat(plain_rest)));
        }
        out.push(t.fg(denom, Token::Accent));
        out
    }
}

/// Token count: 1_000_000 -> "1M", 28_000 -> "28K", 590 -> "590".
fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        let m = n as f64 / 1_000_000.0;
        if m.fract() == 0.0 {
            format!("{}M", m as u64)
        } else {
            format!("{m:.1}M")
        }
    } else if n >= 1000 {
        let k = n as f64 / 1000.0;
        if k.fract() == 0.0 {
            format!("{}K", k as u64)
        } else {
            format!("{k:.1}K")
        }
    } else {
        format!("{n}")
    }
}
