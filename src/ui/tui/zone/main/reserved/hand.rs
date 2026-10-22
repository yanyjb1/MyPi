//! The temp hand's contract: report on arrival, read its own keys, draw its own rows.
//!
//! One hand is on duty at a time; its height is fixed from arrival until it leaves.

use ratatui::text::Line;

use crate::ui::tui::action::{Claim, Key};
use crate::ui::tui::zone::{Ctx, Event};

/// What a hand asks for when it takes the desk.
#[derive(Debug, Clone, Copy)]
pub struct Report {
    /// Rows it wants. Fixed from arrival to departure.
    pub rows: u16,
    /// Keys it borrows: while it is on duty these go to it and to nobody else.
    pub keys: &'static [Claim],
    /// It wants the editor's text and cursor after every edit.
    pub text: bool,
    /// It wants the cursor's column after every draw.
    pub col: bool,
}

/// What a hand hands back to the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Replace {
        from: usize,
        to: usize,
        text: String,
    },
}

pub trait Hand {
    /// On arrival, before the first report.
    fn arrived(&mut self, ctx: &Ctx);

    /// Rows, keys, feeds. Read once, then kept until the hand leaves.
    fn report(&self) -> Report;

    /// Has what it needs to report? The desk waits for this before freezing the height.
    fn ready(&self) -> bool {
        true
    }

    /// Still on duty? `false` = the desk drops it.
    fn stays(&self) -> bool {
        true
    }

    fn on_event(&mut self, _ev: Event, _ctx: &Ctx) {}

    fn text(&mut self, _text: &str, _cursor: usize) {}

    fn col(&mut self, _col: u16) {}

    /// One borrowed key. `Some` = something for the editor.
    fn on_key(&mut self, _key: &Key) -> Option<Reply> {
        None
    }

    fn draw(&self, ctx: &Ctx, rows: u16) -> Vec<Line<'static>>;

    /// Leaving: put your own state away.
    fn left(&mut self);
}

/// Idle height: one blank row, so the layout never jumps on arrival or departure.
pub const IDLE_ROWS: u16 = 1;
