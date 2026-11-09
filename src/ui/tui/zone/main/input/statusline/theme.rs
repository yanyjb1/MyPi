//! Statusline colors: one snapshot per frame, built from the live accent.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::ui::tui::theme::{Accent, fixed};

/// Every color a statusline deco may ask for: one entry per rendering decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Frames, the gauge's used part, the model pill, the session pill, connectors.
    Accent,
    /// Money and the `?N` / `+N` counts.
    Gold,
    /// The idle `π`.
    Muted,
    /// The capsule band the left group and the session pill sit on.
    Capsule,
    /// Plain foreground text: the working-directory path.
    Text,
    /// Clean git branch.
    GitClean,
    /// Dirty git branch.
    GitDirty,
    /// The ` > ` connector between left decos.
    Sep,
}

/// Every slot, in index order.
pub const ALL_SLOTS: [Slot; 8] = [
    Slot::Accent,
    Slot::Gold,
    Slot::Muted,
    Slot::Capsule,
    Slot::Text,
    Slot::GitClean,
    Slot::GitDirty,
    Slot::Sep,
];

impl Slot {
    const fn idx(self) -> usize {
        match self {
            Slot::Accent => 0,
            Slot::Gold => 1,
            Slot::Muted => 2,
            Slot::Capsule => 3,
            Slot::Text => 4,
            Slot::GitClean => 5,
            Slot::GitDirty => 6,
            Slot::Sep => 7,
        }
    }
}

/// A per-frame snapshot of the colors the statusline is allowed to use.
#[derive(Debug, Clone, Copy)]
pub struct StatusTheme {
    colors: [Color; ALL_SLOTS.len()],
}

impl StatusTheme {
    /// Snapshot one frame's colors from the live accent.
    pub fn new(accent: Accent) -> Self {
        let mut colors = [Color::Reset; ALL_SLOTS.len()];
        colors[Slot::Accent.idx()] = accent.color();
        colors[Slot::Gold.idx()] = fixed::STATUS_COST;
        colors[Slot::Muted.idx()] = fixed::MUTED;
        colors[Slot::Capsule.idx()] = fixed::USER_BG;
        colors[Slot::Text.idx()] = fixed::STATUS_PATH;
        colors[Slot::GitClean.idx()] = fixed::STATUS_GIT_CLEAN;
        colors[Slot::GitDirty.idx()] = fixed::STATUS_GIT_DIRTY;
        colors[Slot::Sep.idx()] = fixed::STATUS_SEP;
        Self { colors }
    }

    /// The resolved value of one slot.
    pub fn get(&self, tok: Slot) -> Color {
        self.colors[tok.idx()]
    }

    // ---- span builders: the only place a statusline span is styled ----

    /// Capsule body: the capsule background plus `fg`.
    pub fn capsule(&self, text: impl Into<String>, fg: Color) -> Span<'static> {
        Span::styled(text.into(), Style::new().bg(self.get(Slot::Capsule)).fg(fg))
    }

    /// Capsule background only — the capsule's breathing room.
    pub fn on_capsule(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().bg(self.get(Slot::Capsule)))
    }

    /// A left-group connector: capsule background, separator foreground.
    pub fn sep(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(
            text.into(),
            Style::new()
                .bg(self.get(Slot::Capsule))
                .fg(self.get(Slot::Sep)),
        )
    }

    /// Foreground only, transparent background.
    pub fn fg(&self, text: impl Into<String>, tok: Slot) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.get(tok)))
    }

    /// No style at all: default foreground, transparent background.
    pub fn plain(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new())
    }
}
