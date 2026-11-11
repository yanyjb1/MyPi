//! The usage gauge — `>--8.3%-----:1M`. The row's flexible deco.
//!
//! `~` prefixes the percentage when the count is an estimate; the dashes carry no mark.

use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use super::super::Deco;
use super::super::theme::{Slot, StatusTheme};
use crate::ui::tui::zone::main::input::Ctx;

#[derive(Debug, Default)]
pub struct Usage;

pub(super) fn make() -> Box<dyn Deco> {
    Box::new(Usage)
}

impl Deco for Usage {
    fn render(&self, t: &StatusTheme, budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        // A measured count wins; before one lands, the archive's floor stands in, marked;
        // before either, the bar is a placeholder that reads zero rather than vanishing.
        let (now, estimated) = if let Some(tokens) = ctx.data.context.tokens {
            (tokens, ctx.data.context.estimated)
        } else if let Some(floor) = ctx.data.baseline {
            (floor, true)
        } else {
            (0, false)
        };
        let limit = ctx.data.limit;
        let pct = if limit > 0 {
            (now.min(limit) as f64 / limit as f64 * 100.0).min(100.0)
        } else {
            0.0
        };
        let pct_txt = if estimated {
            format!("~{pct:.1}%")
        } else {
            format!("{pct:.1}%")
        };
        // An unconfigured window still gets a denominator slot: `?` is a fact too.
        let denom = match limit {
            0 => ":?".to_string(),
            limit => format!(":{}", fmt_tokens(limit)),
        };
        // Cells outside the overhead belong to the dashes.
        let overhead = 1 + pct_txt.width() + denom.width();
        let cells = budget.unwrap_or(overhead + 8).max(overhead + 1);
        let dash_w = cells - overhead;
        // The percentage sits between the dashes: accent counts the used share,
        // plain the rest. At zero the first accent cell stays.
        let used = (pct as usize * dash_w / 100).min(dash_w);
        let used = if dash_w > 0 { used.max(1) } else { 0 };
        let plain_rest = dash_w - used;

        let mut out = vec![t.fg(">", Slot::Accent)];
        if used > 0 {
            out.push(t.fg("-".repeat(used), Slot::Accent));
        }
        out.push(t.fg(pct_txt, Slot::Accent));
        if plain_rest > 0 {
            out.push(t.plain("-".repeat(plain_rest)));
        }
        out.push(t.fg(denom, Slot::Accent));
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
