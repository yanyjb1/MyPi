//! The local section: an append-only slot sequence, and the number/block map.
//!
//! Numbers below zero are the past, counting back from -1; zero is the first message of this
//! conversation, and new ones count up from there. A number is handed out once and never moves.

use std::collections::VecDeque;

use crate::ui::session::{Level, Life};

/// What one slot holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotKind {
    /// The number is taken; the block id has not arrived.
    Empty,
    /// One block of the conversation, named upstream by this id.
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

/// The local section. Only ever appended to.
#[derive(Default)]
pub struct Local {
    /// Numbers below zero, newest first: `past[0]` is -1.
    past: VecDeque<Slot>,
    /// Numbers from zero up, in order.
    now: Vec<Slot>,
}

impl Local {
    /// The next number for a new slot, counting up from zero.
    pub fn append(&mut self, kind: SlotKind) -> i64 {
        let num = self.now.len() as i64;
        self.now.push(Slot { num, kind });
        num
    }

    /// Give ids to the slots that were waiting for them, oldest first.
    pub fn land(&mut self, ids: &[i64]) {
        let mut ids = ids.iter();
        for slot in self.now.iter_mut() {
            if !matches!(slot.kind, SlotKind::Empty) {
                continue;
            }
            let Some(id) = ids.next() else { break };
            slot.kind = SlotKind::Block(*id);
        }
    }

    /// A short-lived line dies: its slot keeps its number and draws nothing.
    pub fn kill_short_lived(&mut self) {
        for slot in self.now.iter_mut().chain(self.past.iter_mut()) {
            if let SlotKind::Notice {
                life: Life::Short, ..
            } = slot.kind
            {
                slot.kind = SlotKind::Next;
            }
        }
    }

    /// Another conversation: the whole section goes.
    pub fn clear(&mut self) {
        self.past.clear();
        self.now.clear();
    }

    pub fn at(&self, num: i64) -> Option<&Slot> {
        if num < 0 {
            self.past.get((-num - 1) as usize)
        } else {
            self.now.get(num as usize)
        }
    }

    /// Oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Slot> {
        self.past.iter().rev().chain(self.now.iter())
    }

    pub fn len(&self) -> usize {
        self.past.len() + self.now.len()
    }

    /// The newest number handed out, if any.
    pub fn newest(&self) -> Option<i64> {
        if let Some(slot) = self.now.last() {
            return Some(slot.num);
        }
        self.past.front().map(|slot| slot.num)
    }

    /// The oldest number still held.
    pub fn oldest(&self) -> Option<i64> {
        if let Some(slot) = self.past.back() {
            return Some(slot.num);
        }
        self.now.first().map(|slot| slot.num)
    }

    pub fn is_empty(&self) -> bool {
        self.past.is_empty() && self.now.is_empty()
    }

    pub fn block_of(&self, num: i64) -> Option<i64> {
        match self.at(num)?.kind {
            SlotKind::Block(id) => Some(id),
            _ => None,
        }
    }

    pub fn num_of(&self, block: i64) -> Option<i64> {
        self.iter().find_map(|slot| match slot.kind {
            SlotKind::Block(id) if id == block => Some(slot.num),
            _ => None,
        })
    }
}
