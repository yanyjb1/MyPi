//! The finished conversation: -N..-1 for history, 0..M for this session, append only.

use std::collections::{HashMap, VecDeque};

use super::super::product::Product;
use super::content::Block;
use super::slot::{Held, Slot, SlotKind};

/// The two sides of one tool exchange, by block id.
#[derive(Default, Clone, Copy)]
pub(crate) struct Pair {
    pub request: Option<i64>,
    pub result: Option<i64>,
}

/// Numbers below zero hold what the database had; zero up holds this front end's own run.
#[derive(Default)]
pub struct Immutable {
    past: VecDeque<Slot>,
    now: Vec<Slot>,
    index: HashMap<i64, i64>,
    pairs: HashMap<String, Pair>,
    clock: u64,
    /// Slots that hold content right now: the budget counts these.
    held: usize,
}

impl Immutable {
    /// The next admission stamp: the eviction order.
    pub fn stamp(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

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
    pub fn kill_short(&mut self) -> Vec<i64> {
        let mut gone = Vec::new();
        for slot in self.past.iter_mut().chain(self.now.iter_mut()) {
            if let SlotKind::Notice {
                life: super::slot::Life::Short,
                ..
            } = slot.kind
            {
                slot.kind = SlotKind::Next;
                gone.push(slot.num);
            }
        }
        gone
    }

    /// A rewind: the slot keeps its number and stops holding a block.
    pub fn dead(&mut self, num: i64) {
        let (id, filled) = match self.at(num).map(|slot| &slot.kind) {
            Some(SlotKind::Block { id, held }) => (Some(*id), held.is_some()),
            _ => (None, false),
        };
        if let Some(id) = id {
            self.index.remove(&id);
        }
        if filled {
            self.held = self.held.saturating_sub(1);
        }
        if let Some(slot) = self.at_mut(num) {
            slot.kind = SlotKind::Next;
        }
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.now.clear();
        self.index.clear();
        self.pairs.clear();
        self.clock = 0;
        self.held = 0;
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

    pub fn at_mut(&mut self, num: i64) -> Option<&mut Slot> {
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

    /// Whether this session's own blocks have positions.
    pub fn has_now(&self) -> bool {
        !self.now.is_empty()
    }

    /// The content on this slot, when it is in hand.
    pub fn block(&self, num: i64) -> Option<&Block> {
        match &self.at(num)?.kind {
            SlotKind::Block {
                held: Some(held), ..
            } => Some(&held.block),
            _ => None,
        }
    }

    /// The product this slot's block handed over, when it was built.
    pub fn product(&self, num: i64) -> Option<&Product> {
        match &self.at(num)?.kind {
            SlotKind::Block {
                held: Some(held), ..
            } => held.product.as_ref(),
            _ => None,
        }
    }

    /// Keep the built product on the slot.
    pub fn set_product(&mut self, num: i64, product: Product) {
        if let Some(SlotKind::Block {
            held: Some(held), ..
        }) = self.at_mut(num).map(|slot| &mut slot.kind)
        {
            held.product = Some(product);
        }
    }

    pub fn drop_product(&mut self, num: i64) {
        if let Some(SlotKind::Block {
            held: Some(held), ..
        }) = self.at_mut(num).map(|slot| &mut slot.kind)
        {
            held.product = None;
        }
    }

    pub fn drop_products(&mut self) {
        for slot in self.past.iter_mut().chain(self.now.iter_mut()) {
            if let SlotKind::Block {
                held: Some(held), ..
            } = &mut slot.kind
            {
                held.product = None;
            }
        }
    }

    /// The block content arrives: the slot it belongs to takes it.
    pub fn bind(&mut self, id: i64, block: Block) {
        let Some(num) = self.num_of(id) else {
            return;
        };
        let at = self.stamp();
        let empty = matches!(
            self.at(num).map(|slot| &slot.kind),
            Some(SlotKind::Block { held: None, .. })
        );
        if empty {
            self.held += 1;
        }
        let Some(slot) = self.at_mut(num) else {
            return;
        };
        if let SlotKind::Block { held, .. } = &mut slot.kind {
            *held = Some(Box::new(Held {
                block,
                product: None,
                at,
            }));
        }
    }

    /// The exchange this block belongs to, as this region knows it.
    pub fn pair(&self, call: &str) -> Option<Pair> {
        self.pairs.get(call).copied()
    }

    pub fn pair_mut(&mut self, call: &str) -> &mut Pair {
        self.pairs.entry(call.to_string()).or_default()
    }

    /// Drop content until the region holds `budget` blocks. Exempt blocks stay, and an exchange
    /// is kept or dropped as one: a card draws both halves.
    pub fn evict(&mut self, budget: usize, keep: impl Fn(i64, i64) -> bool) -> Vec<i64> {
        if self.held <= budget {
            return Vec::new();
        }
        let mut candidates: Vec<(u64, i64, i64)> = self
            .past
            .iter()
            .chain(self.now.iter())
            .filter_map(|slot| match &slot.kind {
                SlotKind::Block {
                    id,
                    held: Some(held),
                } => Some((held.at, slot.num, *id)),
                _ => None,
            })
            .collect();
        candidates.sort_unstable();
        let mut out = Vec::new();
        for (_, num, id) in candidates {
            if self.held <= budget {
                break;
            }
            if self.block(num).is_none() || keep(id, num) {
                continue;
            }
            let partner = self
                .partner(num, id)
                .filter(|(_, onum)| self.block(*onum).is_some());
            if let Some((other, onum)) = partner
                && keep(other, onum)
            {
                continue;
            }
            for victim in std::iter::once(num).chain(partner.map(|(_, other)| other)) {
                if let Some(SlotKind::Block { held, .. }) =
                    self.at_mut(victim).map(|slot| &mut slot.kind)
                {
                    *held = None;
                }
                self.held -= 1;
                out.push(victim);
            }
        }
        out
    }

    /// The other half of this block's exchange, when it has content.
    fn partner(&self, num: i64, id: i64) -> Option<(i64, i64)> {
        let call = self.block(num)?.call_id.as_deref()?;
        let pair = self.pairs.get(call)?;
        let other = if pair.request == Some(id) {
            pair.result
        } else {
            pair.request
        }?;
        let at = self.num_of(other)?;
        (at != num).then_some((other, at))
    }

    fn remember(&mut self, num: i64, kind: &SlotKind) {
        if let SlotKind::Block { id, held } = kind {
            self.index.insert(*id, num);
            if held.is_some() {
                self.held += 1;
            }
        }
    }
}
