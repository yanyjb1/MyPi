//! Esc: the one key that means two things, one after the other.

use std::time::{Duration, Instant};

use crate::ui::tui::action::Key;
use crate::ui::tui::zone::main::input::InputZone;
use crate::ui::tui::zone::main::input::exit::ExitRequest;

// How long the second Esc waits for its partner.
const DOUBLE: Duration = Duration::from_millis(400);

impl InputZone {
    // `true` = consumed.
    pub(super) fn route_esc(&mut self, _key: &Key) -> bool {
        if self
            .esc_armed
            .take()
            .is_some_and(|at| at.elapsed() < DOUBLE)
        {
            self.pending_exits.push(ExitRequest::OpenTree);
            return true;
        }
        self.esc_armed = Some(Instant::now());
        false
    }
}
