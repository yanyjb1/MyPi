//! The `/usage` panel: one of the desk's hands.
//!
//! The numbers are computed by the server; the front end only lays them out.
//!
//! ```text
//!  Total Token: 2.4M, exactly: 2,440,000
//!  Cache Hit Rate: 83.33%
//!
//!  Model              Total      Input      Output     CacheRead  CacheWrite
//! >model-alpha        ¥4.62      ¥1.20      ¥3.40      ¥0.02      ¥0.00
//! ```
//!
//! The panel never enters the history, the store or the context: Esc closes it.

use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::ui::tui::action::{Key, KeyCode, Mods};
use crate::ui::tui::theme::Palette;
use crate::ui::tui::zone::Claim;
use crate::ui::tui::zone::main::num;
use crate::ui::tui::zone::main::reserved::hand::{Hand, Report};

/// No modifiers held.
const PLAIN: Mods = Mods {
    ctrl: false,
    alt: false,
    shift: false,
};

/// Keys the panel borrows: Esc closes, arrows pick a row.
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

/// One model's share of the bill.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    address: String,
    rounds: u64,
    prompt: u64,
    completion: u64,
    cached: u64,
    written: u64,
    share: f64,
    currency: String,
    total: Option<f64>,
    input: Option<f64>,
    output: Option<f64>,
    read: Option<f64>,
    write: Option<f64>,
}

#[derive(Debug, Default)]
pub struct UsagePanel {
    /// The selected table row.
    selected: usize,
    open: bool,
    /// The numbers arrived at least once.
    loaded: bool,
    rows: Vec<Row>,
}

impl UsagePanel {
    /// The view's usage arrived; remember the rows.
    pub fn take(&mut self, answer: &Value) {
        let Some(groups) = answer.get("groups").and_then(Value::as_array) else {
            return;
        };
        self.rows = groups.iter().map(row_of).collect();
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        self.loaded = true;
    }

    fn close(&mut self) {
        self.open = false;
    }

    /// Move the selection, wrapping around.
    fn step(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.rem_euclid(self.rows.len() as isize) as usize;
    }

    /// Three header rows plus one row per model.
    fn height(&self) -> usize {
        (3 + self.rows.len().max(1)).min(MAX_ROWS)
    }

    fn draw_panel(&self, width: u16, rows: usize, p: &Palette) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        if !self.loaded {
            out.push(Line::from(p.muted("正在读取…")));
            return out;
        }
        let prompt: u64 = self.rows.iter().map(|r| r.prompt).sum();
        let completion: u64 = self.rows.iter().map(|r| r.completion).sum();
        let cached: u64 = self.rows.iter().map(|r| r.cached).sum();
        let total = prompt + completion;
        let hit = if prompt == 0 {
            0.0
        } else {
            cached as f64 / prompt as f64 * 100.0
        };
        out.push(Line::from(vec![
            p.accent_span("Total Token: "),
            Span::raw(num::compact(total)),
            Span::raw(format!(", exactly: {}", num::exact(total))),
        ]));
        out.push(Line::from(format!("Cache Hit Rate: {hit:.2}%")));
        out.push(Line::from(""));
        out.push(Line::from(
            "Model              Total      Input      Output     CacheRead  CacheWrite",
        ));
        let limit = rows.saturating_sub(out.len()).max(1);
        for (i, row) in self.rows.iter().take(limit).enumerate() {
            let marker = if i == self.selected { ">" } else { " " };
            let symbol = row.currency.as_str();
            let money = |v: Option<f64>| num::money(v.unwrap_or(0.0), symbol);
            let line = format!(
                "{marker}{:<18} {:<10} {:<10} {:<10} {:<10} {:<10}",
                row.address,
                money(row.total),
                money(row.input),
                money(row.output),
                money(row.read),
                money(row.write),
            );
            out.push(Line::from(if i == self.selected {
                p.accent_span(line)
            } else {
                Span::raw(line)
            }));
        }
        let _ = (width, self.rows.first().map(|r| r.share));
        out
    }
}

impl Hand for UsagePanel {
    /// Open right away: the numbers arrive on their own, a frame or two later.
    fn arrived(&mut self) {
        self.open = true;
    }

    fn report(&self) -> Report {
        Report {
            rows: self.height() as u16,
            keys: PANEL_KEYS,
        }
    }

    /// Nothing to report until the numbers arrived.
    fn ready(&self) -> bool {
        self.loaded
    }

    fn stays(&self) -> bool {
        self.open
    }

    fn on_key(&mut self, key: &Key) -> Option<crate::ui::tui::zone::main::desk::Reply> {
        match key.code {
            KeyCode::Esc => self.close(),
            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            _ => {}
        }
        None
    }

    fn draw(&self, width: u16, rows: u16, palette: &Palette) -> Vec<Line<'static>> {
        self.draw_panel(width, usize::from(rows), palette)
    }

    fn left(&mut self) {
        self.close();
    }
}

/// One `groups` row, decoded.
fn row_of(group: &Value) -> Row {
    let price = group.get("price");
    let money = |key: &str| price.and_then(|p| p.get(key)).and_then(Value::as_f64);
    Row {
        address: group
            .get("address")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        rounds: group.get("rounds").and_then(Value::as_u64).unwrap_or(0),
        prompt: group
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        completion: group
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cached: group
            .get("cached_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        written: group
            .get("cache_write_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        share: group.get("share").and_then(Value::as_f64).unwrap_or(0.0),
        currency: price
            .and_then(|p| p.get("currency"))
            .and_then(Value::as_str)
            .map(|name| match name {
                "CNY" => "¥".to_string(),
                "USD" => "$".to_string(),
                other => other.to_string(),
            })
            .unwrap_or_default(),
        total: money("total"),
        input: money("input"),
        output: money("output"),
        read: money("cache_read"),
        write: money("cache_write"),
    }
}
