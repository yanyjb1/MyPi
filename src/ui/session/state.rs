//! What the front end knows about the attached session: the attachment, its flags, the outbox.

use std::collections::VecDeque;

use super::event::Event;

/// The session's own state, plus the queue of events waiting for the front end.
#[derive(Default)]
pub struct State {
    attached: Option<i64>,
    busy: bool,
    new_born: bool,
    refused: bool,
    init: Option<i64>,
    watching: Option<i64>,
    outbox: VecDeque<Event>,
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
        self.init = None;
        self.watching = None;
        self.outbox.clear();
    }

    pub fn detach(&mut self) {
        self.attached = None;
        self.busy = false;
        self.new_born = false;
        self.refused = false;
        self.init = None;
        self.watching = None;
        self.outbox.clear();
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

    /// The session whose first frame is in flight.
    pub fn init(&self) -> Option<i64> {
        self.init
    }

    pub fn set_init(&mut self, session: Option<i64>) {
        self.init = session;
    }

    /// The session this connection already watches.
    pub fn watching(&self) -> Option<i64> {
        self.watching
    }

    pub fn set_watching(&mut self, session: Option<i64>) {
        self.watching = session;
    }

    /// Queue one event for the front end, in arrival order.
    pub fn push(&mut self, event: Event) {
        self.outbox.push_back(event);
    }

    /// Take everything queued; the order is the arrival order.
    pub fn drain(&mut self) -> Vec<Event> {
        self.outbox.drain(..).collect()
    }
}
