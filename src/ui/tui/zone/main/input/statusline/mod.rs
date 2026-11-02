//! One row of the statusline: the deco table, the width engine, the theme.

mod deco;
mod layout;
mod theme;

pub use theme::StatusTheme;

use std::time::Duration;

use ratatui::text::Line;

use serde_json::Value;

use super::Ctx;

/// Which group of the row a deco renders in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Before the flexible group; ` > ` between neighbours.
    Left,
    /// The flexible group: takes the leftover width.
    Flex,
    /// After the flexible group; `< ` before every deco.
    Right,
}

impl Side {
    /// Sort key: the engine visits groups in this order.
    pub(super) const fn rank(self) -> u8 {
        match self {
            Side::Left => 0,
            Side::Flex => 1,
            Side::Right => 2,
        }
    }
}

/// Decos at this priority are never dropped; a flexible deco's narrowest form
/// counts as part of this tier.
pub const PRIORITY_BASE: u8 = 100;

/// One row of the table: everything about a deco that is not behaviour.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    /// Which group of the row.
    pub side: Side,
    /// Position inside its group, ascending = left to right.
    pub order: u8,
    /// Omission priority: higher survives longer.
    pub priority: u8,
    /// Builds it.
    pub make: fn(&Setup) -> Box<dyn Deco>,
}

/// What a deco may read before the first frame.
#[derive(Debug, Clone)]
pub struct Setup {
    /// `config.ui.tui.zone.main.input.deco.gitScanInterval`, this until then.
    pub git_scan: Duration,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            git_scan: deco::git::DEFAULT_SCAN,
        }
    }
}

/// What a deco does. Where it sits and what it costs live in its [`Entry`].
pub trait Deco {
    /// One bus event; ignore the ones you do not care about.
    fn on_event(&mut self, _event: &str, _content: &Value, _ctx: &Ctx) {}

    /// `budget` is `None` while measuring (return the narrowest form), else the
    /// cells allocated. An empty run hides the deco and its connector.
    fn render(
        &self,
        t: &StatusTheme,
        budget: Option<usize>,
        ctx: &Ctx,
    ) -> Vec<ratatui::text::Span<'static>>;
}

/// The row, built from [`deco::ROW`] and sorted by `(side, order)`.
pub struct StatusLine {
    rows: Vec<(&'static Entry, Box<dyn Deco>)>,
}

impl StatusLine {
    /// Build the standard row.
    pub fn new(setup: &Setup) -> Self {
        let mut rows: Vec<(&'static Entry, Box<dyn Deco>)> = deco::ROW
            .iter()
            .map(|entry| (entry, (entry.make)(setup)))
            .collect();
        rows.sort_by_key(|(entry, _)| (entry.side.rank(), entry.order));
        Self { rows }
    }

    /// Broadcast one event to every deco.
    pub fn on_event(&mut self, event: &str, content: &Value, ctx: &Ctx) {
        for (_, deco) in &mut self.rows {
            deco.on_event(event, content, ctx);
        }
    }

    /// Render the row. Exactly `width` cells, always.
    pub fn render(&self, t: &StatusTheme, width: u16, ctx: &Ctx) -> Line<'static> {
        layout::render_line(&self.rows, t, width, ctx)
    }
}
