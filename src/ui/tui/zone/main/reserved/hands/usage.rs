//! The `/usage` panel: one of the desk's hands.
//!
//! The numbers are computed by the server; the front end only lays them out.
//! Three width tiers: full headers, abbreviated headers, model + total only.
//! Tab flips exact figures to per-model shares; the middle tier pins the share face.
//!
//! The panel never enters the history, the store or the context: Esc closes it.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::ui::tui::action::{Key, KeyCode, Mods};
use crate::ui::tui::theme::{Accent, fixed};
use crate::ui::tui::zone::Claim;
use crate::ui::tui::zone::main::input::{Currency, Lang};
use crate::ui::tui::zone::main::num;
use crate::ui::tui::zone::main::reserved::hand::{Hand, Report};

/// No modifiers held.
const PLAIN: Mods = Mods {
    ctrl: false,
    alt: false,
    shift: false,
};

/// Keys the panel borrows: Esc closes, arrows pick a row, Tab flips the face.
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
    Claim::Key(Key {
        code: KeyCode::Tab,
        mods: PLAIN,
    }),
];

/// The panel's row cap.
const MAX_ROWS: usize = 24;

/// Below this width the full column set cannot fit.
const FULL_MIN_WIDTH: u16 = 96;

/// One model's share of the bill.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    address: String,
    display: Option<String>,
    provider: Option<String>,
    rounds: u64,
    prompt: u64,
    completion: u64,
    cached: u64,
    written: u64,
    total_cny: f64,
    total_usd: f64,
}

impl Row {
    /// The narrow-tier label: the user's display name over the raw address.
    fn label(&self) -> String {
        match (&self.display, &self.provider) {
            (Some(display), Some(provider)) => format!("{display} ({provider})"),
            _ => self.address.clone(),
        }
    }
}

/// One tier of the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Full,
    Short,
    Bare,
}

#[derive(Debug, Default)]
pub struct UsagePanel {
    /// The selected table row.
    selected: usize,
    open: bool,
    /// The numbers arrived at least once.
    loaded: bool,
    rows: Vec<Row>,
    lang: Lang,
    currency: Currency,
    /// `true` = the per-model share face; `false` = exact token counts.
    brief: bool,
    /// Display names by address, from the model entries the session answered.
    displays: BTreeMap<String, (String, String)>,
}

impl UsagePanel {
    /// The view's usage arrived; remember the rows, and ask for the entries
    /// whose display facts the session has not named yet.
    pub fn take(&mut self, answer: &Value) -> Vec<crate::ui::tui::zone::Request> {
        let Some(groups) = answer.get("groups").and_then(Value::as_array) else {
            return Vec::new();
        };
        self.rows = groups.iter().map(row_of).collect();
        for row in &mut self.rows {
            if let Some((display, provider)) = self.displays.get(&row.address) {
                row.display = Some(display.clone());
                row.provider = Some(provider.clone());
            }
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        self.loaded = true;
        self.rows
            .iter()
            .filter(|row| !self.displays.contains_key(&row.address) && !row.address.is_empty())
            .map(|row| crate::ui::tui::zone::Request::Config {
                key: format!("models.model.{}", row.address),
            })
            .collect()
    }

    /// The display language the fixed rows and headers follow.
    pub fn set_lang(&mut self, lang: Lang) {
        self.lang = lang;
    }

    /// The currency the panel prices in.
    pub fn set_currency(&mut self, currency: Currency) {
        self.currency = currency;
    }

    /// One model entry landed: its display name joins the pool.
    pub fn display(&mut self, entry: &Value) {
        let Some(address) = entry.get("address").and_then(Value::as_str) else {
            return;
        };
        let Some(name) = entry.get("name").and_then(Value::as_str) else {
            return;
        };
        let provider = entry
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or_default();
        self.displays.insert(
            address.to_string(),
            (name.to_string(), provider.to_string()),
        );
        for row in &mut self.rows {
            if row.address == address {
                row.display = Some(name.to_string());
                row.provider = Some(provider.to_string());
            }
        }
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

    fn tier(&self, width: u16) -> Tier {
        if width >= FULL_MIN_WIDTH {
            Tier::Full
        } else if self.lang == Lang::En && width >= 40 {
            Tier::Short
        } else {
            Tier::Bare
        }
    }

    /// The fixed rows, one per model, and a blank line.
    fn height(&self) -> usize {
        let fixed = if self.rows.is_empty() { 2 } else { 3 };
        (fixed + self.rows.len()).min(MAX_ROWS)
    }

    /// The totals line and the hit-rate line, in the panel's face.
    fn fixed_rows(&self, tier: Tier, brief: bool) -> Vec<String> {
        let prompt: u64 = self.rows.iter().map(|r| r.prompt).sum();
        let completion: u64 = self.rows.iter().map(|r| r.completion).sum();
        let cached: u64 = self.rows.iter().map(|r| r.cached).sum();
        let total = prompt + completion;
        let rounds: u64 = self.rows.iter().map(|r| r.rounds).sum();
        let hit = if prompt == 0 {
            0.0
        } else {
            cached as f64 / prompt as f64 * 100.0
        };
        let (token_word, hit_word) = match self.lang {
            Lang::En => ("Total Token", "Cache Hit Rate"),
            Lang::Zh => ("总词元", "缓存命中率"),
        };
        let figure = |n: u64| {
            if brief || tier != Tier::Full {
                num::compact(n, self.lang)
            } else {
                num::exact(n, self.lang)
            }
        };
        let rounds_text = if self.lang == Lang::Zh {
            format!("{rounds} 轮")
        } else {
            format!("{rounds} rounds")
        };
        vec![
            format!("{token_word}: {}", figure(total)),
            format!("{hit_word}: {hit:.2}%   {rounds_text}"),
        ]
    }

    /// The whole panel in one face; `brief` only plays in the full tier.
    fn draw_as(&self, width: u16, rows: usize, brief: bool, accent: &Accent) -> Vec<Line<'static>> {
        let tier = self.tier(width);
        let mut out: Vec<Line<'static>> = Vec::new();
        if !self.loaded {
            out.push(Line::from(Span::styled(
                match self.lang {
                    Lang::En => "Reading…",
                    Lang::Zh => "正在读取…",
                },
                Style::new().fg(fixed::MUTED),
            )));
            return out;
        }
        if self.rows.is_empty() {
            out.push(Line::from(Span::styled(
                match self.lang {
                    Lang::En => "No usage yet",
                    Lang::Zh => "还没有用量",
                },
                Style::new().fg(fixed::MUTED),
            )));
            return out;
        }
        for text in self.fixed_rows(tier, brief) {
            out.push(Line::from(text));
        }
        out.push(Line::from(""));
        if tier == Tier::Bare {
            self.draw_bare(&mut out, accent);
        } else {
            self.draw_table(&mut out, tier, brief, accent, rows);
        }
        out
    }

    /// The bare tier: `display (provider)` and the total, nothing else.
    fn draw_bare(&self, out: &mut Vec<Line<'static>>, accent: &Accent) {
        let symbol = self.currency.symbol();
        for (i, row) in self.rows.iter().enumerate() {
            let money = match self.currency {
                Currency::Cny => row.total_cny,
                Currency::Usd => row.total_usd,
            };
            let text = format!("{:<28} {}", row.label(), num::money(money, symbol));
            out.push(Line::from(if i == self.selected {
                accent.span(text)
            } else {
                Span::raw(text)
            }));
        }
    }

    /// The full and short tiers: headers, then one line per model.
    fn draw_table(
        &self,
        out: &mut Vec<Line<'static>>,
        tier: Tier,
        brief: bool,
        accent: &Accent,
        rows: usize,
    ) {
        let headers: [&str; 6] = match (tier, self.lang) {
            (Tier::Full, Lang::En) => [
                "Model",
                "Total",
                "Input",
                "Output",
                "CacheRead",
                "CacheWrite",
            ],
            (Tier::Full, Lang::Zh) => ["模型", "合计", "输入", "输出", "缓存读", "缓存写"],
            (Tier::Short, _) => ["Model", "Total", "In", "Ou", "CR", "CW"],
            (Tier::Bare, _) => unreachable!("bare has its own path"),
        };
        out.push(Line::from(headers.join("  ")));
        let limit = rows.saturating_sub(out.len()).max(1);
        let symbol = self.currency.symbol();
        for (i, row) in self.rows.iter().take(limit).enumerate() {
            let label = if tier == Tier::Full {
                row.address.clone()
            } else {
                row.label()
            };
            let line = if brief {
                let denom = (row.prompt + row.completion).max(1) as f64;
                format!(
                    "{}  {}  {:.1}%  {:.1}%  {:.1}%  {:.1}%",
                    label,
                    num::money(self.total_of(row), symbol),
                    row.prompt as f64 / denom * 100.0,
                    row.completion as f64 / denom * 100.0,
                    row.cached as f64 / denom * 100.0,
                    row.written as f64 / denom * 100.0,
                )
            } else {
                format!(
                    "{}  {}  {}  {}  {}  {}",
                    label,
                    num::money(self.total_of(row), symbol),
                    num::compact(row.prompt, self.lang),
                    num::compact(row.completion, self.lang),
                    num::compact(row.cached, self.lang),
                    num::compact(row.written, self.lang),
                )
            };
            out.push(Line::from(if i == self.selected {
                accent.span(line)
            } else {
                Span::raw(line)
            }));
        }
    }

    /// The row's total in the panel's currency.
    fn total_of(&self, row: &Row) -> f64 {
        match self.currency {
            Currency::Cny => row.total_cny,
            Currency::Usd => row.total_usd,
        }
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
            KeyCode::Tab => self.brief = !self.brief,
            _ => {}
        }
        None
    }

    fn draw(&self, width: u16, rows: u16, accent: &Accent) -> Vec<Line<'static>> {
        // The middle tier pins the share face; Tab only plays in the full tier.
        let brief = if self.tier(width) == Tier::Full {
            self.brief
        } else {
            true
        };
        self.draw_as(width, usize::from(rows), brief, accent)
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
        display: None,
        provider: None,
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
        total_cny: money("cny").unwrap_or(0.0),
        total_usd: money("usd").unwrap_or(0.0),
    }
}
