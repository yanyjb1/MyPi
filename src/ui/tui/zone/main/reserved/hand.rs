//! The temp hand's contract: report on arrival, read its own keys, draw its own rows.
//!
//! One hand is on duty at a time; its height is fixed from arrival until it leaves.

use ratatui::text::Line;
use serde_json::Value;

use crate::ui::tui::action::Key;
use crate::ui::tui::theme::Accent;
use crate::ui::tui::zone::Claim;

use super::super::desk::Reply;
use super::pools::Pools;

/// What a hand asks for when it takes the desk.
#[derive(Debug, Clone, Copy)]
pub struct Report {
    /// Rows it wants. Fixed from arrival to departure.
    pub rows: u16,
    /// Keys it borrows: while it is on duty these go to it and to nobody else.
    pub keys: &'static [Claim],
}

pub trait Hand {
    /// On arrival, before the first report.
    fn arrived(&mut self);

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

    /// One bus event. Unknown names are ignored.
    fn on_event(&mut self, _event: &str, _content: &Value) {}

    /// The workspace a path completes against.
    fn place(&mut self, _cwd: &str) {}

    /// The name pools, once the session has answered.
    fn pools(&mut self, _pools: &Pools) {}

    fn text(&mut self, _text: &str, _cursor: usize) {}

    fn col(&mut self, _col: u16) {}

    /// One borrowed key. `Some` = something for the editor.
    fn on_key(&mut self, _key: &Key) -> Option<Reply> {
        None
    }

    fn draw(&self, width: u16, rows: u16, accent: &Accent) -> Vec<Line<'static>>;

    /// Leaving: put your own state away.
    fn left(&mut self);
}

/// Idle height: one blank row, so the layout never jumps on arrival or departure.
pub const IDLE_ROWS: u16 = 1;
