//! Esc: interrupt, and a second tap inside the window asks for the tree.

use std::time::{Duration, Instant};

use super::exit::ExitRequest;
use super::zone::InputZone;
use crate::ui::tui::action::Key;

/// Two un-lent Esc this close together ask for the node tree.
const ESC_WINDOW: Duration = Duration::from_millis(300);

impl InputZone {
    /// Esc, when nothing borrowed it: it interrupts, and a second tap within the
    /// window also asks for the node tree.
    pub(super) fn route_esc(&mut self, key: &Key) -> bool {
        if key.mods.alt || key.mods.ctrl {
            return false;
        }
        self.enter_armed = None;
        self.pending_exits.push(ExitRequest::Interrupt);
        let now = Instant::now();
        match self.esc_armed.take() {
            Some(at) if now.duration_since(at) < ESC_WINDOW => {
                self.pending_exits.push(ExitRequest::OpenTree);
            }
            _ => self.esc_armed = Some(now),
        }
        true
    }
}
