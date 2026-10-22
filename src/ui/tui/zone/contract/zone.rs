//! The two-level contract: App talks to zones, Main talks to sub-zones.

use super::claim::Claim;
use super::ctx::{Ctx, TermSize};
use super::handover::Handover;
use super::request::Request;
use crate::ui::tui::action::Action;
use crate::ui::tui::event::Event;

/// A zone the app can show. The app never looks inside one.
pub trait Zone {
    fn deliver_event(&mut self, ev: Event, ctx: &Ctx);

    /// An action. Returns `Some` when the zone gives the screen up.
    fn deliver_action(&mut self, a: &Action, ctx: &Ctx) -> Option<Handover>;

    /// The front end is on a conversation: every zone hears it.
    fn attach(&mut self, _session: i64, _ctx: &Ctx) {}

    /// The front end left the conversation.
    fn detach(&mut self, _ctx: &Ctx) {}

    /// The zone is losing the screen; this is the only memo window besides takeover.
    fn hand_over(&mut self, _ctx: &Ctx) {}

    /// The zone is taking the screen; this is the only memo window besides handover.
    fn take_over(&mut self, _ctx: &Ctx) {}

    /// Draw this zone. Row allocation inside it is its own business.
    fn render(&mut self, ctx: &Ctx) -> Vec<ratatui::text::Line<'static>>;

    /// Where the hardware cursor goes, in terminal coordinates.
    fn cursor(&self) -> Option<(u16, u16)> {
        None
    }

    /// Take what this zone asks the session to do.
    fn take_requests(&mut self) -> Vec<Request> {
        Vec::new()
    }
}

/// A sub-zone of the chat zone: claims input, receives events, draws itself.
pub trait SubZone {
    fn claims(&self) -> &'static [Claim];

    fn deliver_event(&mut self, ev: Event, ctx: &Ctx);

    /// An action claimed by this sub-zone. `true` = consumed.
    fn deliver_action(&mut self, a: &Action, ctx: &Ctx) -> bool;

    fn attach(&mut self, _session: i64, _ctx: &Ctx) {}

    fn detach(&mut self, _ctx: &Ctx) {}

    /// The keys this sub-zone borrows right now.
    fn borrowed(&self) -> &'static [Claim] {
        &[]
    }

    /// Rows this sub-zone needs, given the terminal size.
    fn request_height(&self, term: TermSize) -> u16;

    /// Rows the zone allocated, plus the terminal size for wrapping and hit tests.
    fn assign(&mut self, term: TermSize, rows: u16);

    /// The zone's row broadcast. `true` = consumed.
    fn on_height_changed(&mut self, rows: u16) -> bool;

    /// Session actions this sub-zone produced since the last take.
    fn take_requests(&mut self) -> Vec<Request> {
        Vec::new()
    }
}
