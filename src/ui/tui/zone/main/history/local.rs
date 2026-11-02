//! The local section: an append-only run of numbered slots.
//!
//! Numbers below zero are the past, counting back from -1; zero is the first message of this
//! conversation, and new ones count up from there. A planned rewind is the one thing that takes
//! slots away: the tail goes, and its numbers are handed out again.

use std::collections::{HashMap, VecDeque};

/// How loud one of this front end's own lines is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// How long a line of this front end's own stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Life {
    /// Dies when the next lasting thing lands.
    Short,
    /// Stays for the rest of the conversation.
    Long,
}

/// What one slot holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotKind {
    /// One block, named upstream by this id.
    Block(i64),
    /// A line this front end has to say. It lives here and never goes upstream.
    Notice {
        text: String,
        level: Level,
        life: Life,
    },
    /// A short-lived line after its death: no rows, no gap.
    Next,
}

/// One slot: a number, and what sits on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub num: i64,
    pub kind: SlotKind,
}

/// The local section.
#[derive(Default)]
pub struct Local {
    /// Numbers below zero, oldest first: the last entry is -1.
    past: VecDeque<Slot>,
    /// Numbers from zero up, in order.
    now: Vec<Slot>,
    /// Block id to number, so a per-frame walk does not scan the sequence.
    index: HashMap<i64, i64>,
}

impl Local {
    /// The next number above zero.
    pub fn push_now(&mut self, kind: SlotKind) -> i64 {
        let num = self.now.len() as i64;
        self.remember(num, &kind);
        self.now.push(Slot { num, kind });
        num
    }

    /// One more step into the past: -1, then -2, and so on.
    pub fn push_older(&mut self, kind: SlotKind) -> i64 {
        let num = -((self.past.len() as i64) + 1);
        self.remember(num, &kind);
        self.past.push_front(Slot { num, kind });
        num
    }

    /// A short-lived line dies: its slot keeps its number and draws nothing.
    pub fn kill_short_lived(&mut self) {
        for slot in self.past.iter_mut().chain(self.now.iter_mut()) {
            if let SlotKind::Notice {
                life: Life::Short, ..
            } = slot.kind
            {
                slot.kind = SlotKind::Next;
            }
        }
    }

    /// A rewind: the slots from `len` up go, and their numbers are free again.
    pub fn truncate_now(&mut self, len: usize) {
        while self.now.len() > len {
            if let Some(slot) = self.now.pop()
                && let SlotKind::Block(id) = slot.kind
            {
                self.index.remove(&id);
            }
        }
    }

    /// Another conversation: the whole section goes.
    pub fn clear(&mut self) {
        self.past.clear();
        self.now.clear();
        self.index.clear();
    }

    pub fn at(&self, num: i64) -> Option<&Slot> {
        if num < 0 {
            let at = self.past.len() as i64 + num;
            if at < 0 {
                None
            } else {
                self.past.get(at as usize)
            }
        } else {
            self.now.get(num as usize)
        }
    }

    /// Oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Slot> {
        self.past.iter().chain(self.now.iter())
    }

    /// The newest number handed out, if any.
    pub fn newest(&self) -> Option<i64> {
        self.now
            .last()
            .map(|slot| slot.num)
            .or_else(|| self.past.back().map(|slot| slot.num))
    }

    /// The oldest number still held.
    pub fn oldest(&self) -> Option<i64> {
        self.past
            .front()
            .map(|slot| slot.num)
            .or_else(|| self.now.first().map(|slot| slot.num))
    }

    pub fn num_of(&self, block: i64) -> Option<i64> {
        self.index.get(&block).copied()
    }

    fn remember(&mut self, num: i64, kind: &SlotKind) {
        if let SlotKind::Block(id) = kind {
            self.index.insert(*id, num);
        }
    }
}
