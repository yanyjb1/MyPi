//! The `/usage` panel: one of the reserved area's hands.
//!
//! The numbers are computed by the server; the front end only lays them out.
//!
//! Fixed layout: three header rows, then one row per model.
//!
//! ```text
//!  Total Token: 2.4M, exactly: 2,440,000
//!  Cache Hit Rate: 83.33%
//!
//!  Model              Total      Input      Output     CacheRead  CacheWrite
//! >model-alpha        ¥4.62      ¥1.20      ¥3.40      ¥0.02      ¥0.00
//!  model-beta         ¥1.74      ¥0.50      ¥1.20      ¥0.04      ¥0.00
//! ```
//!
//! The first two rows are the whole session; the table is per model: fixed column
//! widths, right aligned, no rules.
//!
//! The panel never enters the history, the store or the context: Esc closes it.

use ratatui::text::{Line, Span};

use crate::ui::tui::UsageAlign;
use crate::ui::tui::action::{Key, KeyCode, Mods};
use crate::ui::tui::theme::Palette;
use crate::ui::tui::zone::main::num;
use crate::ui::tui::zone::main::reserved::hand::{Hand, Report};
use crate::ui::tui::zone::{Claim, Ctx, Reply};

/// No modifiers held.
const PLAIN: Mods = Mods {
    ctrl: false,
    alt: false,
    shift: false,
};

/// Keys the panel borrows: Esc closes, arrows pick a row, Tab switches the number unit.
const PANEL_KEYS: &[Claim] = &[
    Claim::Key(Key {
        code: KeyCode::Esc,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::Up,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::Down,
        mods: PLAIN,
    }),
];

/// The panel's row cap.
const MAX_ROWS: usize = 24;
/// Columns the selection marker takes.
const MARKER_W: usize = 1;

/// Layout steps, widest first: `(name column, money column, header names)`.
///
/// The five money columns never shrink. Real width per step = `name + 5 * money`,
/// plus the marker: 73 / 58 / 49 / 42 / 35 columns. Narrower terminals clip the
/// row ends.
const LAYOUTS: [(usize, usize, [&str; 5]); 5] = [
    (
        18,
        11,
        ["Total", "Input", "Output", "CacheRead", "CacheWrite"],
    ),
    (13, 9, ["Total", "Input", "Output", "CRead", "CWrite"]),
    (9, 8, ["Total", "In", "Out", "CRead", "CWrite"]),
    (7, 7, ["Tot", "In", "Out", "CRe", "CWr"]),
    (5, 6, ["Tot", "In", "Out", "CR", "CW"]),
];

#[derive(Debug, Default)]
pub struct UsagePanel {
    /// The selected table row.
    selected: usize,
    /// Which side the block hugs; horizontal padding only.
    align: UsageAlign,
    open: bool,
    /// The numbers arrived at least once.
    loaded: bool,
    /// How many model rows the view holds; layout only, never the numbers.
    body_rows: usize,
}

impl UsagePanel {
    /// Open (or replace) the panel; one more `/usage` recomputes and reopens it.
    pub fn show(&mut self) {
        self.selected = 0;
        self.open = true;
    }

    /// The view's usage arrived; remember only how many rows to reserve.
    pub fn sync(&mut self, view: &crate::ui::session::View<'_>) {
        let Some(usage) = view.usage else { return };
        self.body_rows = usage.groups.len();
        self.loaded = true;
    }

    /// Where the block sits horizontally, told once at assembly.
    pub fn set_align(&mut self, align: UsageAlign) {
        self.align = align;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Three header rows plus one row per model; an empty session still gets one.
    fn rows(&self) -> usize {
        (3 + self.body_rows.max(1)).min(MAX_ROWS)
    }

    /// Move the selection, wrapping around.
    fn step(&mut self, delta: isize) {
        let n = self.body_rows;
        if n == 0 {
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.rem_euclid(n as isize) as usize;
    }

    fn draw_panel(&self, ctx: &Ctx, rows: usize, p: &Palette) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        let Some(usage) = ctx.view.usage else {
            out.push(Line::from(p.muted("正在读取…")));
            return out;
        };
        let groups = &usage.groups;
        let prompt: u64 = groups.iter().map(|g| g.prompt_tokens).sum();
        let completion: u64 = groups.iter().map(|g| g.completion_tokens).sum();
        let cached: u64 = groups.iter().map(|g| g.cached_tokens).sum();
        let total = prompt + completion;
        let hit = if prompt == 0 {
            0.0
        } else {
            cached as f64 / prompt as f64 * 100.0
        };
        out.push(Line::from(vec![
            p.accent_span("Total Token: "),
            Span::raw(num::compact(total, usage.lang)),
            Span::raw(format!(", exactly: {}", num::exact(total, usage.lang))),
        ]));
        out.push(Line::from(format!("Cache Hit Rate: {hit:.2}%")));
        out.push(Line::from(""));
        out.push(Line::from(
            "Model              Total      Input      Output     CacheRead  CacheWrite",
        ));
        let limit = rows.saturating_sub(out.len()).max(1);
        for (i, g) in groups.iter().take(limit).enumerate() {
            let marker = if i == self.selected { ">" } else { " " };
            let cost = g.cost.as_ref();
            let total = cost.map(|c| c.input + c.output + c.cache_read + c.cache_write);
            let symbol = g.currency.as_deref().unwrap_or("");
            let money = |v: Option<f64>| num::money(v.unwrap_or(0.0), symbol);
            let line = format!(
                "{marker}{:<18} {:<10} {:<10} {:<10} {:<10} {:<10}",
                g.id,
                money(total),
                money(cost.map(|c| c.input)),
                money(cost.map(|c| c.output)),
                money(cost.map(|c| c.cache_read)),
                money(cost.map(|c| c.cache_write)),
            );
            out.push(Line::from(if i == self.selected {
                p.accent_span(line)
            } else {
                Span::raw(line)
            }));
        }
        if out.len() < rows {
            out.push(Line::from(""));
        }
        let _ = self.align;
        out
    }
}

impl Hand for UsagePanel {
    /// Open right away: the numbers arrive on their own, a frame or two later.
    fn arrived(&mut self, _ctx: &Ctx) {
        self.open = true;
    }

    fn report(&self) -> Report {
        Report {
            rows: self.rows() as u16,
            keys: PANEL_KEYS,
            text: false,
            col: false,
        }
    }

    /// Nothing to report until the numbers arrived.
    fn ready(&self) -> bool {
        self.loaded
    }

    fn stays(&self) -> bool {
        self.open
    }

    fn on_key(&mut self, key: &Key) -> Option<Reply> {
        match key.code {
            KeyCode::Esc => self.close(),
            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            _ => {}
        }
        None
    }

    fn draw(&self, ctx: &Ctx, rows: u16) -> Vec<Line<'static>> {
        self.draw_panel(ctx, usize::from(rows), &Palette::current())
    }

    fn left(&mut self) {
        self.close();
    }
}
