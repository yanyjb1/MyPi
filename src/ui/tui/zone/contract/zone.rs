//! The two levels: the app talks to major zones, a major zone talks to its sub-zones.

use ratatui::text::Line;

use super::claim::Claim;
use super::kind::Kind;
use super::memo::Memo;
use super::outcome::Outcome;
use super::request::Request;
use super::size::TermSize;
use crate::ui::session::{Packet, Structure};
use crate::ui::tui::action::Action;

/// A major zone. The app only ever sees this.
pub trait Zone {
    /// Every input this zone consumes, declared once.
    fn keys(&self) -> Vec<Claim>;

    /// The frame classes this zone handles. `local` never appears here: it is unconditional.
    fn kinds(&self) -> &'static [Kind];

    /// The app gives the screen to this zone: the size, and the memo the last zone left.
    fn start(&mut self, size: TermSize, memo: &Memo) -> Vec<Request>;

    /// The conversation was entered (`Some`) or left (`None`).
    fn session(&mut self, id: Option<i64>) -> Vec<Request>;

    /// One frame the zone declared (or any `local` one).
    fn event(&mut self, packet: &Packet) -> Vec<Request>;

    /// One input, already resolved to this zone.
    fn action(&mut self, action: &Action) -> (Outcome, Vec<Request>);

    /// The structural window, after a `Request::Structure`.
    fn structure(&mut self, snapshot: &Structure) -> Vec<Request>;

    /// This zone's rows for the frame.
    fn render(&mut self) -> Vec<Line<'static>>;

    /// Where the hardware cursor goes, in terminal coordinates.
    fn cursor(&self) -> Option<(u16, u16)> {
        None
    }
}

/// A sub-zone of the chat zone. Only `Main` implements the host side.
pub trait SubZone {
    fn claims(&self) -> &'static [Claim];

    fn kinds(&self) -> &'static [Kind];

    /// One frame this sub-zone declared.
    fn deliver_event(&mut self, packet: &Packet, size: TermSize) -> Vec<Request>;

    /// An action claimed by this sub-zone. `true` = consumed.
    fn deliver_action(&mut self, action: &Action) -> bool;

    fn attach(&mut self, session: i64, size: TermSize);

    fn detach(&mut self);

    /// The structural window, after a `Request::Structure`.
    fn structure(&mut self, _snapshot: &Structure) {}

    /// The keys this sub-zone borrows right now.
    fn borrowed(&self) -> &'static [Claim] {
        &[]
    }

    /// Rows this sub-zone needs, given the terminal size.
    fn request_height(&self, term: TermSize) -> u16;

    /// Rows the host allocated, plus the terminal size.
    fn assign(&mut self, term: TermSize, rows: u16);

    /// The host's row broadcast. `true` = consumed.
    fn on_height_changed(&mut self, rows: u16) -> bool;
}
