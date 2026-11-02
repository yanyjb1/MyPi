//! The chat zone's key table: which seat consumes which input.
//!
//! Rows come from the sub-zones: each hands over its own claims, so nobody else names
//! another seat's keys. Borrowed rows (the reserved zone's hand) sit in front of the static
//! ones and are checked first.

use crate::ui::tui::action::Action;
use crate::ui::tui::zone::{Claim, matches};

/// Which sub-zone consumes an input. `Main` owns this: no other level needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seat {
    History,
    Input,
    Reserved,
}

/// The static table: each sub-zone's own claims, in seat order.
const TABLE: &[(Seat, &[Claim])] = &[
    (Seat::History, super::history::zone::CLAIMS),
    (Seat::Input, super::input::InputZone::CLAIMS),
    (Seat::Reserved, super::reserved::ReservedZone::CLAIMS),
];

/// Who claims this action: borrowed rows first, then the static table.
pub fn resolve(a: &Action, borrowed: &Borrowed) -> Option<Seat> {
    borrowed.resolve(a).or_else(|| {
        TABLE
            .iter()
            .find(|(_, claims)| claims.iter().any(|claim| matches(claim, a)))
            .map(|(seat, _)| *seat)
    })
}

/// Rows handed over at run time, checked before the static table.
#[derive(Debug, Default)]
pub struct Borrowed {
    rows: Vec<(Claim, Seat)>,
}

impl Borrowed {
    /// Who holds this action among the borrowed rows.
    pub fn resolve(&self, a: &Action) -> Option<Seat> {
        self.rows
            .iter()
            .find(|(claim, _)| matches(claim, a))
            .map(|(_, seat)| *seat)
    }

    /// Record a loan. Same seat, same claims: nothing moves.
    pub fn lend(&mut self, claims: &[Claim], seat: Seat) {
        if self.rows.iter().filter(|(_, held)| *held == seat).count() == claims.len()
            && claims
                .iter()
                .zip(self.rows.iter().filter(|(_, held)| *held == seat))
                .all(|(want, (have, _))| want == have)
        {
            return;
        }
        self.release(seat);
        self.rows.extend(claims.iter().map(|c| (*c, seat)));
    }

    /// Return every key this seat borrowed.
    pub fn release(&mut self, seat: Seat) {
        self.rows.retain(|(_, held)| *held != seat);
    }

    /// Drop every loan.
    pub fn clear(&mut self) {
        self.rows.clear();
    }
}
