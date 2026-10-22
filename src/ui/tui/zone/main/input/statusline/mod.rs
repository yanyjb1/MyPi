//! One row of the statusline: the deco table, the width engine, the theme.

pub mod deco;
mod layout;
pub mod theme;

pub use theme::{ALL_TOKENS, ColorPolicy, StatusTheme, Token};

use std::time::Duration;

use ratatui::text::Line;

use crate::ui::tui::zone::{Ctx, Event};

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

/// Tokens the engine itself uses, not a deco.
const ENGINE_TOKENS: [Token; 2] = [Token::Accent, Token::Sep];

/// One row of the table: everything about a deco that is not behaviour.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    /// Stable identifier.
    pub id: &'static str,
    /// Which group of the row.
    pub side: Side,
    /// Position inside its group, ascending = left to right.
    pub order: u8,
    /// Omission priority: higher survives longer.
    pub priority: u8,
    /// Tokens this deco reads.
    pub colors: ColorPolicy,
    /// Builds it. May not read `Ctx`.
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
    /// Broadcast event; ignore the ones you do not care about.
    fn on_event(&mut self, _ev: Event, _ctx: &Ctx) {}

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
    pub fn on_event(&mut self, ev: Event, ctx: &Ctx) {
        for (_, deco) in &mut self.rows {
            deco.on_event(ev, ctx);
        }
    }

    /// Render the row. Exactly `width` cells, always.
    pub fn render(&self, t: &StatusTheme, width: u16, ctx: &Ctx) -> Line<'static> {
        layout::render_line(&self.rows, t, width, ctx)
    }

    /// Tokens the table declares plus the engine's own, in `ALL_TOKENS` order.
    pub fn declared_tokens(&self) -> Vec<Token> {
        ALL_TOKENS
            .into_iter()
            .filter(|tok| {
                ENGINE_TOKENS.contains(tok)
                    || self.rows.iter().any(|(entry, _)| match entry.colors {
                        ColorPolicy::Tokens(toks) => toks.contains(tok),
                        ColorPolicy::Fixed | ColorPolicy::Custom => false,
                    })
            })
            .collect()
    }
}
