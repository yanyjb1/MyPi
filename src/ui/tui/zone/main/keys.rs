//! The keys each seat claims: the two static tables, and the rows a hand holds right now.

use crate::ui::tui::action::Action;
use crate::ui::tui::zone::{Claim, matches};

/// The reserved seat sits after the two static ones.
const RESERVED: usize = 2;

/// History and input, in seat order, plus the keys a hand has borrowed.
pub struct Keys {
    lent: &'static [Claim],
    seats: [&'static [Claim]; 2],
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            lent: &[],
            seats: [&[], &[]],
        }
    }
}

impl Keys {
    pub fn new(seats: [&'static [Claim]; 2]) -> Self {
        Self { lent: &[], seats }
    }

    /// A hand takes the desk: its keys go to it and to nobody else.
    pub fn lend(&mut self, claims: &'static [Claim]) {
        self.lent = claims;
    }

    /// The desk is empty: every borrowed key goes back.
    pub fn release(&mut self) {
        self.lent = &[];
    }

    /// The one seat this action belongs to, by declaration.
    pub fn seat(&self, a: &Action) -> Option<usize> {
        if self.lent.iter().any(|claim| matches(claim, a)) {
            return Some(RESERVED);
        }
        self.seats
            .iter()
            .position(|claims| claims.iter().any(|claim| matches(claim, a)))
    }
}
