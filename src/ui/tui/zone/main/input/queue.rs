//! The pending messages: sentences sent while a round was running, in order.
//!
//! The slot on the server always holds the head; every change here keeps it that way.

use std::collections::VecDeque;

use crate::ui::tui::zone::Frame;
use serde_json::json;

/// One sentence the user sent while a round was running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub prefix: Option<String>,
    pub text: String,
    pub suffix: Option<String>,
}

impl Outgoing {
    pub fn new(prefix: Option<String>, text: String) -> Outgoing {
        Outgoing {
            prefix,
            text,
            suffix: None,
        }
    }

    /// The letter that writes this sentence into the slot.
    fn letter(&self, session: i64) -> Frame {
        Frame::new(
            "post",
            "mailbox",
            json!({
                "session": session,
                "prefix": self.prefix,
                "text": self.text,
                "suffix": self.suffix,
            }),
        )
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
    pub fn push(&mut self, msg: Outgoing, session: i64) -> Option<Frame> {
        self.items.push_back(msg);
        self.sync(session)
    }

    /// The loop took the head: drop it, and mirror whatever is next.
    pub fn took(&mut self, session: i64) -> (Option<Outgoing>, Option<Frame>) {
        let taken = self.items.pop_front();
        // The slot is empty now, whatever the queue still holds: the next head goes in.
        self.in_slot = false;
        let letter = self.sync(session);
        (taken, letter)
    }

    /// `Alt+Up`: the newest sentence comes back to the editor; the head stays put.
    pub fn pop(&mut self, session: i64) -> (Option<Outgoing>, Option<Frame>) {
        let taken = self.items.pop_back();
        let letter = self.sync(session);
        (taken, letter)
    }

    /// Nothing is queued: the slot is not ours to hold.
    pub fn clear(&mut self) {
        self.items.clear();
        self.in_slot = false;
    }

    /// Make the slot hold the head, or hold nothing.
    fn sync(&mut self, session: i64) -> Option<Frame> {
        match self.items.front() {
            Some(head) if !self.in_slot => {
                self.in_slot = true;
                Some(head.letter(session))
            }
            Some(_) => None,
            None if self.in_slot => {
                self.in_slot = false;
                Some(Frame::new("post", "mailbox", json!({"session": session})))
            }
            None => None,
        }
    }
}
