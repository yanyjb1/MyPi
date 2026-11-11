//! The pending messages: sentences sent while a round was running, in order.
//!
//! The slot on the server always holds the head; every change here keeps it that way.

use std::collections::VecDeque;

use crate::ui::tui::zone::Request;

/// One sentence the user sent while a round was running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub prefix: Option<String>,
    pub text: String,
    pub suffix: Option<String>,
}

impl Outgoing {
    pub fn new(text: String) -> Outgoing {
        Outgoing {
            prefix: None,
            text,
            suffix: None,
        }
    }

    /// The door's frame; the app fills in the session.
    fn frame(&self) -> Request {
        Request::Mailbox {
            prefix: self.prefix.clone(),
            text: self.text.clone(),
            suffix: self.suffix.clone(),
        }
    }
}

/// The queue, head first, plus whether the head is known to be in the server's slot.
#[derive(Debug, Default)]
pub struct Queue {
    items: VecDeque<Outgoing>,
    in_slot: bool,
}

impl Queue {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// A sentence sent while busy: appended, and written if it is the new head.
    pub fn push(&mut self, msg: Outgoing) -> Option<Request> {
        self.items.push_back(msg);
        self.sync()
    }

    /// The loop took the head: drop it, and mirror whatever is next.
    pub fn took(&mut self) -> (Option<Outgoing>, Option<Request>) {
        let taken = self.items.pop_front();
        // The slot is empty now, whatever the queue still holds: the next head goes in.
        self.in_slot = false;
        let request = self.sync();
        (taken, request)
    }

    /// `Alt+Up`: the newest sentence comes back to the editor; the head stays put.
    pub fn pop(&mut self) -> (Option<Outgoing>, Option<Request>) {
        let taken = self.items.pop_back();
        let request = self.sync();
        (taken, request)
    }

    /// Nothing is queued: the slot is not ours to hold.
    pub fn clear(&mut self) {
        self.items.clear();
        self.in_slot = false;
    }

    /// Make the slot hold the head, or hold nothing.
    fn sync(&mut self) -> Option<Request> {
        match self.items.front() {
            Some(head) if !self.in_slot => {
                self.in_slot = true;
                Some(head.frame())
            }
            Some(_) => None,
            None if self.in_slot => {
                self.in_slot = false;
                Some(Request::ClearMailbox)
            }
            None => None,
        }
    }
}
