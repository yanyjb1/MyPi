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

use crate::server::api::call::usage::ModelUsage;
use crate::server::api::types::UsageLang;
use crate::server::config::read::config::UsageAlign;
use crate::ui::tui::action::{Claim, Key, KeyCode, Mods};
use crate::ui::tui::theme::Palette;
use crate::ui::tui::zone::Ctx;
use crate::ui::tui::zone::main::num;
use crate::ui::tui::zone::main::reserved::hand::{Hand, Reply, Report};

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
    Claim::Key(Key {
        code: KeyCode::Tab,
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
    lang: UsageLang,
    groups: Vec<ModelUsage>,
    /// The selected table row.
    selected: usize,
    /// Which side the block hugs; horizontal padding only.
    align: UsageAlign,
    open: bool,
    /// The numbers arrived at least once.
    loaded: bool,
}

impl UsagePanel {
    /// Open (or replace) the panel; one more `/usage` recomputes and reopens it.
    pub fn show(&mut self, lang: UsageLang, groups: Vec<ModelUsage>) {
        self.lang = lang;
        self.groups = groups;
        self.selected = 0;
        self.open = true;
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
        let body = self.groups.len().max(1);
        (3 + body).min(MAX_ROWS)
    }

    /// Move the selection, wrapping around.
    fn step(&mut self, delta: isize) {
        let n = self.groups.len();
        if n == 0 {
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.rem_euclid(n as isize) as usize;
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
            // Tab switches the number unit: the two languages differ in the carry base too.
            KeyCode::Tab => {
                self.lang = match self.lang {
                    UsageLang::En => UsageLang::Zh,
                    UsageLang::Zh => UsageLang::En,
                };
            }
            _ => {}
        }
        None
    }

    fn draw(&self, ctx: &Ctx, rows: u16) -> Vec<Line<'static>> {
        self.draw_panel(ctx.size.cols, usize::from(rows), &Palette::current())
    }

    fn left(&mut self) {
        self.close();
    }
}
