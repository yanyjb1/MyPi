//! The usage gauge — `>----8%----:1M`, the row's only flexible component.
//!
//! Update source: `Usage`, emitted once per frame by the render loop from the
//! cost tracker. The arithmetic stays in `crate::server::ai::pricing`.
//!
//! It owns three things at once, which is why it is one component and not two:
//!
//! - the `>` connector into the gauge (the engine therefore adds no connector
//!   of its own before a flexible component);
//! - the gauge itself: accent dashes for the used share, then the percentage,
//!   then unstyled dashes for the unused share;
//! - the denominator `:1M` — it belongs to the gauge, not to the session name
//!   it sits next to.
//!
//! It absorbs all the slack the row has left, down to its narrowest legal form
//! (`>-8%:1M`). That form is part of the base tier: a long model name can
//! never squeeze the gauge out, it can only squeeze itself out.

use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use crate::tui::zone::main::input::statusline::theme::{ColorPolicy, StatusTheme, Token};
use crate::tui::zone::main::input::statusline::{PRIORITY_BASE, Side, StatusComponent, StatusEvent};

#[derive(Debug, Default)]
pub struct Usage {
    tokens: u64,
    limit: u64,
}

impl StatusComponent for Usage {
    fn id(&self) -> &'static str {
        "usage"
    }

    fn side(&self) -> Side {
        Side::Flex
    }

    fn order(&self) -> u8 {
        0
    }

    fn priority(&self) -> u8 {
        PRIORITY_BASE
    }

    fn colors(&self) -> ColorPolicy {
        ColorPolicy::Tokens(&[Token::Accent])
    }

    fn on_event(&mut self, ev: &StatusEvent<'_>) {
        if let StatusEvent::Usage(u) = ev {
            self.tokens = u.ctx_tokens;
            self.limit = u.ctx_limit;
        }
    }

    fn render(&self, t: &StatusTheme, budget: Option<usize>) -> Vec<Span<'static>> {
        let pct = if self.limit == 0 {
            0
        } else {
            ((self.tokens.min(self.limit) as f64 / self.limit as f64 * 100.0).round() as u64)
                .min(100)
        };
        let pct_txt = format!("{pct}%");
        let denom = format!(":{}", fmt_tokens(self.limit));
        // `>` + dashes + `8%` + dashes + `:1M`：百分数**嵌在**横线里，
        // 它左边那段（用量）是 accent，右边那段（余量）不是。以前百分数
        // 排在横线**外面**，读数看着像「用了 0%，然后 1M」。
        let overhead = 1 + pct_txt.width() + denom.width();
        let cells = budget.unwrap_or(overhead + 8).max(overhead + 1);
        let dash_w = cells - overhead;
        // The percentage sits on the boundary: accent dashes count the used
        // share, unstyled ones the rest.
        // `>` 右边**至少一格** accent：0% 时那格也在（读起来是"刻度起点"），
        // 百分数就落在这格之后。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_counts_use_k_and_m() {
        assert_eq!(fmt_tokens(1_000_000), "1M");
        assert_eq!(fmt_tokens(28_000), "28K");
        assert_eq!(fmt_tokens(590), "590");
        assert_eq!(fmt_tokens(1_500_000), "1.5M");
    }

    /// The gauge fills exactly the budget it is handed, and never goes below
    /// its narrowest form.
    /// 用量为 0 时：`>` 右边**第一格仍是 accent**，百分数紧跟其后。
    ///
    /// 一格 accent 都没有的话，0% 和"这条量表没接上"看起来是一样的。
    #[test]
    fn an_empty_gauge_still_has_its_first_cell_accented() {
        let u = Usage {
            limit: 1_000_000,
            ..Default::default()
        };
        let t = StatusTheme::resolve();
        let spans = u.render(&t, Some(30));
        let text: String = spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.starts_with(">-0%"), "0% 的百分数紧跟第一格横线：{text}");
        assert_eq!(
            spans[1].style.fg,
            Some(t.get(Token::Accent)),
            "第一格横线是 accent：{:?}",
            spans[1]
        );
        assert!(text.ends_with(":1M"), "{text}");
    }

    #[test]
    fn the_gauge_fills_its_budget() {
        let t = StatusTheme::resolve();
        let mut u = Usage::default();
        u.on_event(&StatusEvent::Usage(crate::tui::zone::main::input::statusline::UsageSnapshot {
            total_cost: 0.0,
            ctx_tokens: 500_000,
            ctx_limit: 1_000_000,
            currency_symbol: "$",
            show_cost: false,
        }));
        for budget in [8usize, 9, 20, 41, 100] {
            let spans = u.render(&t, Some(budget));
            let w: usize = spans.iter().map(|s| s.content.width()).sum();
            assert_eq!(w, budget, "budget {budget}");
        }
        // Measurement pass: the narrowest form. `>` + 4 accent dashes（50%
        // 的用量）+ `50%` + 4 余量横线 + `:1M` = 15。
        let min_spans = u.render(&t, None);
        let min: usize = min_spans.iter().map(|s| s.content.width()).sum();
        assert_eq!(min, 15, "`>----50%----:1M`");
        let min_text: String = min_spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(min_text, ">----50%----:1M", "百分数嵌在横线里");
        // Below the minimum the gauge stays at the minimum (the engine's fit
        // check prevents this from ever being asked for).
        let clamped: usize = u
            .render(&t, Some(2))
            .iter()
            .map(|s| s.content.width())
            .sum();
        // 预算低于下限时停在下限（`>` + 一格横线 + 百分数 + `:1M`）；
        // 引擎的 fit 检查保证这种情况不会被真的问到。
        assert_eq!(clamped, 8);
    }
}
