//! The `/usage` panel: one of the desk's hands.
//! The server computes the numbers; this file lays them out.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::ui::tui::action::{Key, KeyCode, Mods};
use crate::ui::tui::text::{display_width, take_width};
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

/// The config key that places the panel's block.
pub const ALIGN_KEY: &str = "config.ui.tui.zone.main.reserved.usageAlign";

/// The panel's row cap, table head included.
const MAX_ROWS: usize = 24;

/// Cells between two columns.
const GAP: usize = 2;

/// Numeric columns after the label: total, input, output, cache read, cache write.
const COLS: usize = 5;

/// Where the block sits in the strip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Align {
    Left,
    #[default]
    Center,
    Right,
}

impl Align {
    /// The config's spelling; anything else leaves the choice alone.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "left" => Some(Self::Left),
            "center" => Some(Self::Center),
            "right" => Some(Self::Right),
            _ => None,
        }
    }

    /// Cells of empty space before the block.
    fn lead(self, width: usize, block: usize) -> usize {
        let slack = width.saturating_sub(block);
        match self {
            Self::Left => 0,
            Self::Center => slack / 2,
            Self::Right => slack,
        }
    }
}

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
    /// The model carries a price entry.
    priced: bool,
    total_cny: f64,
    total_usd: f64,
}

impl Row {
    /// Tokens billed on this model, cache reads included in the prompt.
    fn total(&self) -> u64 {
        self.prompt + self.completion
    }

    /// The narrowed label: the display name over the provider.
    fn named(&self) -> String {
        match (&self.display, &self.provider) {
            (Some(display), Some(provider)) if !provider.is_empty() => {
                format!("{display}({provider})")
            }
            (Some(display), _) => display.clone(),
            _ => self.address.clone(),
        }
    }
}

/// The column plan for one width.
#[derive(Debug, Clone)]
struct Plan {
    /// The label column's width.
    label: usize,
    /// Numeric widths by column; `None` = the column is dropped.
    widths: [Option<usize>; COLS],
    headers: [&'static str; COLS],
    /// Headers and summary words are abbreviated.
    short: bool,
    /// The label shows the display name instead of the address.
    named: bool,
    /// The token columns show percentages.
    percent: bool,
}

impl Plan {
    /// Every cell the block takes, gaps included.
    fn block(&self) -> usize {
        let mut out = self.label;
        let mut cols = 0;
        for width in self.widths.into_iter().flatten() {
            out += width;
            cols += 1;
        }
        out + GAP * cols
    }
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
    align: Align,
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

    /// Where the block sits in the strip.
    pub fn set_align(&mut self, align: Align) {
        self.align = align;
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

    /// Two summary rows, the blank, the head, and one row per model.
    /// An empty bill keeps four rows: the note stands where the head and rows go.
    fn height(&self) -> usize {
        if !self.loaded {
            return 1;
        }
        (4 + self.rows.len()).min(MAX_ROWS)
    }

    /// The header words of one face.
    fn headers(lang: Lang, short: bool) -> [&'static str; COLS] {
        match (lang, short) {
            (Lang::En, false) => ["Total", "Input", "Output", "CacheRead", "CacheWrite"],
            (Lang::En, true) => ["To", "In", "Ou", "CR", "CW"],
            (Lang::Zh, _) => ["合计", "输入", "输出", "缓存读", "缓存写"],
        }
    }

    /// The label column's header.
    fn label_head(&self, short: bool) -> &'static str {
        match (self.lang, short) {
            (Lang::En, true) => "Mo",
            (Lang::En, false) => "Model",
            (Lang::Zh, _) => "模型",
        }
    }

    /// One cell's text, before padding.
    fn cell(&self, row: &Row, col: usize, percent: bool) -> String {
        if col == 0 {
            return match row.priced {
                true => num::money(self.total_of(row), self.currency.symbol()),
                false => "—".to_string(),
            };
        }
        let part = match col {
            1 => row.prompt,
            2 => row.completion,
            3 => row.cached,
            _ => row.written,
        };
        if !percent {
            return num::exact(part, self.lang);
        }
        let denom = row.total();
        let share = match denom {
            0 => 0.0,
            denom => part as f64 / denom as f64 * 100.0,
        };
        format!("{share:.1}%")
    }

    /// Each column's natural width: the widest of its head and its cells.
    fn numeric(&self, percent: bool, headers: [&'static str; COLS]) -> [usize; COLS] {
        let mut out = headers.map(display_width);
        for row in &self.rows {
            for (i, slot) in out.iter_mut().enumerate() {
                *slot = (*slot).max(display_width(&self.cell(row, i, percent)));
            }
        }
        out
    }

    /// The address face's label width.
    fn addr_width(&self) -> usize {
        self.rows
            .iter()
            .map(|row| display_width(&row.address))
            .fold(display_width(self.label_head(false)), usize::max)
    }

    /// The display-name face's label width.
    fn named_width(&self) -> usize {
        self.rows
            .iter()
            .map(|row| display_width(&row.named()))
            .fold(display_width(self.label_head(false)), usize::max)
    }

    /// The plan when the column set keeps the label's natural width.
    fn fitted(
        natural: usize,
        widths: &[usize; COLS],
        keep: [bool; COLS],
        width: usize,
    ) -> Option<usize> {
        let mut sum = 0;
        let mut cols = 0;
        for (i, width) in widths.iter().enumerate() {
            if keep[i] {
                sum += *width;
                cols += 1;
            }
        }
        (sum + GAP * cols + natural <= width).then_some(natural)
    }

    /// The narrowest plan that still draws: the total column, a clipped label.
    fn floor(
        &self,
        widths: &[usize; COLS],
        headers: [&'static str; COLS],
        short: bool,
        width: usize,
    ) -> Plan {
        let mut keep = [false; COLS];
        keep[0] = true;
        let rest = width.saturating_sub(widths[0] + GAP).max(1);
        Plan {
            label: self.named_width().min(rest),
            widths: std::array::from_fn(|i| keep[i].then(|| widths[i])),
            headers,
            short,
            named: true,
            percent: true,
        }
    }

    /// Pick the face and the columns this width holds.
    fn plan(&self, width: usize) -> Plan {
        let width = width.max(1);
        let full = Self::headers(self.lang, false);
        let short = Self::headers(self.lang, true);
        let all = [true; COLS];
        // Room: the address, exact counts, full heads.
        let room = self.numeric(self.brief, full);
        if let Some(label) = Self::fitted(self.addr_width(), &room, all, width) {
            return Plan {
                label,
                widths: room.map(Some),
                headers: full,
                short: false,
                named: false,
                percent: self.brief,
            };
        }
        // Fixed narrowing: display names, percentages, full heads.
        let pct = self.numeric(true, full);
        if let Some(label) = Self::fitted(self.named_width(), &pct, all, width) {
            return Plan {
                label,
                widths: pct.map(Some),
                headers: full,
                short: false,
                named: true,
                percent: true,
            };
        }
        // English heads and summary words shorten together.
        if self.lang == Lang::En {
            let cut = self.numeric(true, short);
            if let Some(label) = Self::fitted(self.named_width(), &cut, all, width) {
                return Plan {
                    label,
                    widths: cut.map(Some),
                    headers: short,
                    short: true,
                    named: true,
                    percent: true,
                };
            }
        }
        // Columns leave: cache write, cache read, output, input.
        let (headers, widths, abbrev) = match self.lang {
            Lang::En => (short, self.numeric(true, short), true),
            Lang::Zh => (full, pct, false),
        };
        let mut keep = [true; COLS];
        for col in [4, 3, 2, 1] {
            keep[col] = false;
            if let Some(label) = Self::fitted(self.named_width(), &widths, keep, width) {
                return Plan {
                    label,
                    widths: std::array::from_fn(|i| keep[i].then(|| widths[i])),
                    headers,
                    short: abbrev,
                    named: true,
                    percent: true,
                };
            }
        }
        self.floor(&widths, headers, abbrev, width)
    }

    /// The two rows above the table.
    fn summary(&self, plan: &Plan, avail: usize) -> Vec<String> {
        let prompt: u64 = self.rows.iter().map(|row| row.prompt).sum();
        let completion: u64 = self.rows.iter().map(|row| row.completion).sum();
        let cached: u64 = self.rows.iter().map(|row| row.cached).sum();
        let rounds: u64 = self.rows.iter().map(|row| row.rounds).sum();
        let total = prompt + completion;
        let hit = match total {
            0 => 0.0,
            total => cached as f64 / total as f64 * 100.0,
        };
        let (token_word, hit_word) = match (self.lang, plan.short) {
            (Lang::En, false) => ("Total Token", "Cache Hit Rate"),
            (Lang::En, true) => ("TT", "CHR"),
            (Lang::Zh, _) => ("总词元", "缓存命中率"),
        };
        let figure = match plan.named {
            true => num::compact(total, self.lang),
            false => num::exact(total, self.lang),
        };
        let counted = match (self.lang, plan.short) {
            (Lang::En, false) => format!("{rounds} rounds"),
            (Lang::En, true) => format!("{rounds} Rds"),
            (Lang::Zh, _) => format!("{rounds} 轮"),
        };
        let long = format!("{hit_word}: {hit:.2}%   {counted}");
        let short = format!("{hit_word}: {hit:.2}%");
        let second = match display_width(&long) <= avail {
            true => long,
            false => short,
        };
        vec![
            clip(&format!("{token_word}: {figure}"), avail),
            clip(&second, avail),
        ]
    }

    /// The head row.
    fn head(&self, plan: &Plan) -> String {
        let mut out = left(self.label_head(plan.short), plan.label);
        for (i, width) in plan.widths.iter().enumerate() {
            let Some(width) = width else {
                continue;
            };
            out.push_str(&" ".repeat(GAP));
            out.push_str(&right(plan.headers[i], *width));
        }
        out
    }

    /// One model row.
    fn line(&self, row: &Row, plan: &Plan) -> String {
        let label = match plan.named {
            true => row.named(),
            false => row.address.clone(),
        };
        let mut out = left(&label, plan.label);
        for (i, width) in plan.widths.iter().enumerate() {
            let Some(width) = width else {
                continue;
            };
            out.push_str(&" ".repeat(GAP));
            out.push_str(&right(&self.cell(row, i, plan.percent), *width));
        }
        out
    }

    /// The row's total in the panel's currency.
    fn total_of(&self, row: &Row) -> f64 {
        match self.currency {
            Currency::Cny => row.total_cny,
            Currency::Usd => row.total_usd,
        }
    }

    fn draw_as(&self, width: u16, rows: usize, accent: &Accent) -> Vec<Line<'static>> {
        let width = usize::from(width).max(1);
        let mut out: Vec<Line<'static>> = Vec::new();
        if !self.loaded {
            let word = match self.lang {
                Lang::En => "Reading…",
                Lang::Zh => "正在读取…",
            };
            out.push(Line::from(Span::styled(
                word,
                Style::new().fg(fixed::MUTED),
            )));
            return out;
        }
        let plan = self.plan(width);
        let lead = self.align.lead(width, plan.block());
        let pad = " ".repeat(lead);
        let avail = width.saturating_sub(lead);
        for text in self.summary(&plan, avail) {
            out.push(Line::from(format!("{pad}{text}")));
        }
        if self.rows.is_empty() {
            let word = match self.lang {
                Lang::En => "No usage yet",
                Lang::Zh => "还没有用量",
            };
            out.push(Line::default());
            out.push(Line::from(vec![
                Span::raw(pad),
                Span::styled(word, Style::new().fg(fixed::MUTED)),
            ]));
            return out;
        }
        out.push(Line::default());
        out.push(Line::from(format!("{pad}{}", self.head(&plan))));
        let room = rows.saturating_sub(out.len());
        for (i, row) in self.rows.iter().take(room).enumerate() {
            let text = format!("{pad}{}", self.line(row, &plan));
            out.push(Line::from(match i == self.selected {
                true => accent.span(text),
                false => Span::raw(text),
            }));
        }
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
            KeyCode::Tab => self.brief = !self.brief,
            _ => {}
        }
        None
    }

    fn draw(&self, width: u16, rows: u16, accent: &Accent) -> Vec<Line<'static>> {
        self.draw_as(width, usize::from(rows), accent)
    }

    fn left(&mut self) {
        self.close();
    }
}

/// A cell cut to `w` cells, the cut marked.
fn clip(text: &str, w: usize) -> String {
    if display_width(text) <= w {
        return text.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let (mut cut, _) = take_width(text, w.saturating_sub(1));
    cut.push('…');
    cut
}

/// A cell left-aligned in `w` cells.
fn left(text: &str, w: usize) -> String {
    let mut out = clip(text, w);
    let pad = w.saturating_sub(display_width(&out));
    out.push_str(&" ".repeat(pad));
    out
}

/// A cell right-aligned in `w` cells.
fn right(text: &str, w: usize) -> String {
    let out = clip(text, w);
    let pad = w.saturating_sub(display_width(&out));
    format!("{}{out}", " ".repeat(pad))
}

/// One `groups` row, decoded.
fn row_of(group: &Value) -> Row {
    let price = group.get("price").filter(|value| !value.is_null());
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
        priced: price.is_some(),
        total_cny: money("cny").unwrap_or(0.0),
        total_usd: money("usd").unwrap_or(0.0),
    }
}
