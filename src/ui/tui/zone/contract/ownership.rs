//! The ownership ledger. Every handover goes through it.

use super::handover::ZoneId;

#[derive(Debug)]
pub struct Ownership {
    showing: Option<ZoneId>,
}

impl Ownership {
    pub fn new(initial: ZoneId) -> Self {
        Self {
            showing: Some(initial),
        }
    }

    /// The zone that shows, or `None` once nobody took over.
    pub fn showing(&self) -> Option<ZoneId> {
        self.showing
    }

    /// Record a handover.
    pub fn register(&mut self, to: Option<ZoneId>) {
        self.showing = to;
    }
}
