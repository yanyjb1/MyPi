//! What the front end knows about the attached session: the attachment, its flags, the outbox.

use std::collections::VecDeque;

use super::client::Arrival;

/// The session's own state, plus the queue of upstream objects waiting for the front end.
#[derive(Default)]
pub struct State {
    attached: Option<i64>,
    busy: bool,
    new_born: bool,
    refused: bool,
    outbox: VecDeque<Arrival>,
}

impl State {
    pub fn new() -> State {
        State::default()
    }

    /// The session it is on; `None` = nothing attached.
    pub fn attached(&self) -> Option<i64> {
        self.attached
    }

    pub fn attach(&mut self, id: i64) {
        self.attached = Some(id);
        self.busy = false;
        self.new_born = false;
        self.refused = false;
    }

    pub fn detach(&mut self) {
        self.attached = None;
        self.busy = false;
        self.new_born = false;
        self.refused = false;
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    /// Take the new-row flag; `true` means this front end just created the row.
    pub fn take_new_born(&mut self) -> bool {
        std::mem::take(&mut self.new_born)
    }

    pub fn set_new_born(&mut self, value: bool) {
        self.new_born = value;
    }

    /// Whether the last write was refused because the session is mid-round.
    pub fn refused(&self) -> bool {
        self.refused
    }

    pub fn set_refused(&mut self, refused: bool) {
        self.refused = refused;
    }

    /// Queue one upstream object for the front end, in arrival order.
    pub fn push(&mut self, arrival: Arrival) {
        self.outbox.push_back(arrival);
    }

    /// Take everything queued; the order is the arrival order.
    pub fn drain(&mut self) -> Vec<Arrival> {
        self.outbox.drain(..).collect()
    }
}
