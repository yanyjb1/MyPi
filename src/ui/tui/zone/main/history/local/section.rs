//! The local section: render order and the content each position holds.

use std::collections::{BTreeMap, HashMap};

use super::content::{Block, Kind as BlockKind};
use super::immutable::Immutable;
use super::mutable::Mutable;
use super::slot::{Level, Life, Slot, SlotKind};

/// Where one render position sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Idx {
    /// A block of the finished conversation: -N..-1, then 0..M.
    Fixed(i64),
    /// A block of the running turn, by position; it moves as the turn grows.
    Turn(u32),
}

/// The two sides of one tool exchange, as far as they are in hand.
#[derive(Default, Clone, Copy)]
struct Pair {
    request: Option<i64>,
    result: Option<i64>,
}

/// The whole conversation, in render order, and the content behind it.
#[derive(Default)]
pub struct Local {
    fixed: Immutable,
    running: Mutable,
    blocks: BTreeMap<i64, Block>,
    /// `call_id` to the ids of the two blocks it ties.
    pairs: HashMap<String, Pair>,
}

impl Local {
    pub fn clear(&mut self) {
        self.fixed.clear();
        self.running.clear();
        self.blocks.clear();
        self.pairs.clear();
    }

    /// Render order goes; the content already in hand stays.
    pub fn reset_positions(&mut self) {
        self.fixed.clear();
        self.running.clear();
    }

    /// One finished block, appended above zero.
    pub fn append(&mut self, block: Block) -> Idx {
        let id = block.id;
        self.put(block);
        Idx::Fixed(self.fixed.append(SlotKind::Block(id)))
    }

    /// One block of the restored history: newest is -1.
    pub fn append_older(&mut self, id: i64) -> Idx {
        Idx::Fixed(self.fixed.append_older(SlotKind::Block(id)))
    }

    /// A line only this front end has to say: it waits at the end of the running turn.
    pub fn notice(&mut self, text: String, level: Level, life: Life) -> Idx {
        if !self.running.is_empty() {
            self.running.notice(text, level, life);
            return Idx::Turn((self.running.len() - 1) as u32);
        }
        self.fixed.kill_short();
        Idx::Fixed(self.fixed.append(SlotKind::Notice { text, level, life }))
    }

    /// A notice that waits past the turn it arrived in.
    pub fn append_notice(&mut self, text: String, level: Level, life: Life) -> Idx {
        Idx::Fixed(self.fixed.append(SlotKind::Notice { text, level, life }))
    }

    pub fn kill_short(&mut self) {
        self.fixed.kill_short();
    }

    /// A rewind: the position keeps its number and draws nothing.
    pub fn dead(&mut self, idx: Idx) {
        let Idx::Fixed(num) = idx else { return };
        let gone = match self.fixed.at(num).map(|slot| &slot.kind) {
            Some(SlotKind::Block(id)) => Some(*id),
            _ => None,
        };
        if let Some(id) = gone {
            self.drop_content(id);
        }
        self.fixed.dead(num);
    }

    /// A block's content goes, and the side it made in its exchange.
    fn drop_content(&mut self, id: i64) {
        let Some(block) = self.blocks.remove(&id) else {
            return;
        };
        let Some(call) = block.call_id.as_deref() else {
            return;
        };
        let Some(pair) = self.pairs.get_mut(call) else {
            return;
        };
        match block.kind {
            BlockKind::ToolRequest => pair.request = None,
            BlockKind::ToolResult => pair.result = None,
            _ => {}
        }
        if pair.request.is_none() && pair.result.is_none() {
            self.pairs.remove(call);
        }
    }

    pub fn block(&self, idx: Idx) -> Option<&Block> {
        match idx {
            Idx::Fixed(num) => match self.fixed.at(num)?.kind {
                SlotKind::Block(id) => self.blocks.get(&id),
                _ => None,
            },
            Idx::Turn(at) => self.running.block(at as usize),
        }
    }

    pub fn slot(&self, idx: Idx) -> Option<&Slot> {
        match idx {
            Idx::Fixed(num) => self.fixed.at(num),
            Idx::Turn(_) => None,
        }
    }

    /// One slot by number.
    pub fn at(&self, num: i64) -> Option<&Slot> {
        self.fixed.at(num)
    }

    /// The result that answers this position, when it is in hand.
    pub fn result_of(&self, idx: Idx) -> Option<&Block> {
        let block = self.block(idx)?;
        if block.kind != BlockKind::ToolRequest {
            return None;
        }
        let call = block.call_id.as_deref()?;
        match idx {
            Idx::Fixed(_) => {
                let result = self.pairs.get(call)?.result?;
                self.blocks.get(&result)
            }
            Idx::Turn(_) => self.running.result_of(call),
        }
    }

    /// The ids of the exchange this block belongs to, its own included.
    pub fn exchange(&self, block: i64) -> [Option<i64>; 2] {
        let call = self
            .blocks
            .get(&block)
            .and_then(|held| held.call_id.as_deref());
        match call.and_then(|call| self.pairs.get(call)) {
            Some(pair) => [pair.request, pair.result],
            None => [Some(block), None],
        }
    }

    /// A block's content, without giving it a slot: it may already have one.
    pub fn put(&mut self, block: Block) {
        if let Some(call) = block.call_id.as_deref() {
            let pair = self.pairs.entry(call.to_string()).or_default();
            match block.kind {
                BlockKind::ToolRequest => pair.request = Some(block.id),
                BlockKind::ToolResult => pair.result = Some(block.id),
                _ => {}
            }
        }
        self.blocks.insert(block.id, block);
    }

    /// Whether this block's content is in hand.
    pub fn has(&self, id: i64) -> bool {
        self.blocks.contains_key(&id)
    }

    /// Whether the held copy of this block differs from the new one.
    pub fn differs(&self, block: &Block) -> bool {
        self.blocks.get(&block.id).is_some_and(|old| old != block)
    }

    /// A slot for a block whose content is already in hand.
    pub fn append_id(&mut self, id: i64) -> Idx {
        Idx::Fixed(self.fixed.append(SlotKind::Block(id)))
    }

    /// Numbers of the slots whose block the conversation no longer owns.
    pub fn unowned(&self, owns: impl Fn(i64) -> bool) -> Vec<Idx> {
        self.fixed
            .iter()
            .filter_map(|slot| match slot.kind {
                SlotKind::Block(id) if !owns(id) => Some(Idx::Fixed(slot.num)),
                _ => None,
            })
            .collect()
    }

    /// Drop the content the conversation no longer owns.
    pub fn retain(&mut self, owns: impl Fn(i64) -> bool) {
        self.blocks.retain(|id, _| owns(*id));
        let blocks = &self.blocks;
        self.pairs.retain(|_, pair| {
            pair.request = pair.request.filter(|id| blocks.contains_key(id));
            pair.result = pair.result.filter(|id| blocks.contains_key(id));
            pair.request.is_some() || pair.result.is_some()
        });
    }

    /// The position this block id sits on, if it has one.
    pub fn idx_of(&self, block: i64) -> Option<Idx> {
        self.fixed.num_of(block).map(Idx::Fixed)
    }

    pub fn num_of(&self, block: i64) -> Option<i64> {
        self.fixed.num_of(block)
    }

    /// The newest finished position.
    pub fn newest(&self) -> Option<i64> {
        self.fixed.newest()
    }

    /// The oldest finished position.
    pub fn oldest(&self) -> Option<i64> {
        self.fixed.oldest()
    }

    /// The newest block id whose content is in hand.
    pub fn max_id(&self) -> i64 {
        self.blocks.keys().next_back().copied().unwrap_or(0)
    }

    /// The newest user message in hand, for the input box after a rewind.
    pub fn last_user_text(&self) -> Option<String> {
        self.blocks
            .values()
            .rev()
            .find(|block| block.kind == BlockKind::User)
            .and_then(|block| block.content.clone().or_else(|| block.text.clone()))
    }

    pub fn running(&self) -> &Mutable {
        &self.running
    }

    pub fn running_mut(&mut self) -> &mut Mutable {
        &mut self.running
    }
}
