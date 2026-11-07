//! The finished conversation: -N..-1 for history, 0..M for this session, append only.

use std::collections::{HashMap, VecDeque};

use super::slot::{Slot, SlotKind};

/// Numbers below zero hold what the database had; zero up holds this front end's own run.
#[derive(Default)]
pub struct Immutable {
    past: VecDeque<Slot>,
    now: Vec<Slot>,
    index: HashMap<i64, i64>,
}

impl Immutable {
    /// One more slot above zero.
    pub fn append(&mut self, kind: SlotKind) -> i64 {
        let num = self.now.len() as i64;
        self.remember(num, &kind);
        self.now.push(Slot { num, kind });
        num
    }

    /// One more step into the past: -1, then -2, and so on.
    pub fn append_older(&mut self, kind: SlotKind) -> i64 {
        let num = -((self.past.len() as i64) + 1);
        self.remember(num, &kind);
        self.past.push_front(Slot { num, kind });
        num
    }

    /// A short line expires: its slot keeps its number and draws nothing.
    pub fn kill_short(&mut self) {
        for slot in self.past.iter_mut().chain(self.now.iter_mut()) {
            if let SlotKind::Notice {
                life: super::slot::Life::Short,
                ..
            } = slot.kind
            {
                slot.kind = SlotKind::Next;
            }
        }
    }

    /// A rewind: the slot keeps its number and stops holding a block.
    pub fn dead(&mut self, num: i64) {
        let id = match self.at(num).map(|slot| &slot.kind) {
            Some(SlotKind::Block(id)) => Some(*id),
            _ => None,
        };
        if let Some(id) = id {
            self.index.remove(&id);
        }
        if let Some(slot) = self.at_mut(num) {
            slot.kind = SlotKind::Next;
        }
    }

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

    fn at_mut(&mut self, num: i64) -> Option<&mut Slot> {
        if num < 0 {
            let at = self.past.len() as i64 + num;
            if at < 0 {
                None
            } else {
                self.past.get_mut(at as usize)
            }
        } else {
            self.now.get_mut(num as usize)
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
