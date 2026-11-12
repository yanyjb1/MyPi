//! The local section: render order and the content each position holds.

use super::content::{Block, Kind as BlockKind};
use super::immutable::Immutable;
use super::mutable::{Entry, Mutable};
use super::slot::{Held, Level, Life, Slot, SlotKind};
use crate::ui::tui::zone::main::history::product::{self, Opts, Product, Shape};

/// Where one render position sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Idx {
    /// A block of the finished conversation: -N..-1, then 0..M.
    Fixed(i64),
    /// A block of the running turn, by position; it moves as the turn grows.
    Turn(u32),
}

/// The whole conversation, in render order, with the content behind it.
#[derive(Default)]
pub struct Local {
    fixed: Immutable,
    running: Mutable,
}

impl Local {
    pub fn clear(&mut self) {
        self.fixed.clear();
        self.running.clear();
    }

    /// One finished block, appended above zero.
    pub fn append(&mut self, block: Block) -> Idx {
        self.note_pair(&block);
        let id = block.id;
        let at = self.fixed.stamp();
        Idx::Fixed(self.fixed.append(SlotKind::Block {
            id,
            held: Some(Box::new(Held {
                block,
                product: None,
                at,
            })),
        }))
    }

    /// The exchange index follows the content: a request or a result names its half.
    fn note_pair(&mut self, block: &Block) {
        let Some(call) = block.call_id.clone() else {
            return;
        };
        let pair = self.fixed.pair_mut(&call);
        match block.kind {
            BlockKind::ToolRequest => pair.request = Some(block.id),
            BlockKind::ToolResult => pair.result = Some(block.id),
            _ => {}
        }
    }

    /// One block of the restored history: newest is -1. Its content comes later.
    pub fn append_older(&mut self, id: i64) -> Idx {
        Idx::Fixed(self.fixed.append_older(SlotKind::Block { id, held: None }))
    }

    /// A line only this front end has to say: it enters the mutable region like everything else.
    pub fn notice(&mut self, text: String, level: Level, life: Life) -> Idx {
        Idx::Turn(self.running.notice(text, level, life) as u32)
    }

    /// A notice that waits past the turn it arrived in.
    pub fn append_notice(&mut self, text: String, level: Level, life: Life) -> Idx {
        Idx::Fixed(self.fixed.append(SlotKind::Notice { text, level, life }))
    }

    pub fn kill_short(&mut self) -> Vec<i64> {
        self.fixed.kill_short()
    }

    /// A rewind: the position keeps its number and draws nothing.
    pub fn dead(&mut self, idx: Idx) {
        let Idx::Fixed(num) = idx else { return };
        self.fixed.dead(num);
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

    /// Where a result's card is drawn: its request's position, while both sit in the running turn.
    pub fn card_at(&self, idx: Idx) -> Option<Idx> {
        let Idx::Turn(at) = idx else { return None };
        let block = self.running.block(at as usize)?;
        if block.kind != BlockKind::ToolResult {
            return None;
        }
        let call = block.call_id.as_deref()?;
        self.running.request_at(call).map(|at| Idx::Turn(at as u32))
    }

    /// The ids of the exchange this block belongs to, its own included.
    pub fn exchange(&self, block: i64) -> [Option<i64>; 2] {
        let call = self
            .num_of(block)
            .and_then(|num| self.fixed.block(num))
            .and_then(|held| held.call_id.as_deref());
        match call.and_then(|call| self.fixed.pair(call)) {
            Some(pair) => [pair.request, pair.result],
            None => [Some(block), None],
        }
    }

    /// A block's content: it takes the slot its id already has.
    pub fn put(&mut self, block: Block) {
        self.note_pair(&block);
        self.fixed.bind(block.id, block);
    }

    /// The receipt's own numbering: a position per id, in range order, content still to come.
    pub fn reserve(&mut self, ranges: &[[i64; 2]]) {
        for [lo, hi] in ranges {
            for id in *lo..=*hi {
                if self.num_of(id).is_none() {
                    self.fixed.append(SlotKind::Block { id, held: None });
                }
            }
        }
    }

    /// Whether this block's content is in hand.
    pub fn has(&self, id: i64) -> bool {
        self.num_of(id)
            .is_some_and(|num| self.fixed.block(num).is_some())
    }

    /// Whether this position holds a block, content or not.
    pub fn holds_block(&self, idx: Idx) -> bool {
        match idx {
            Idx::Fixed(num) => matches!(
                self.fixed.at(num).map(|slot| &slot.kind),
                Some(SlotKind::Block { .. })
            ),
            Idx::Turn(at) => self.running.block(at as usize).is_some(),
        }
    }

    /// Whether the held copy of this block differs from the new one.
    pub fn differs(&self, block: &Block) -> bool {
        self.num_of(block.id)
            .and_then(|num| self.fixed.block(num))
            .is_some_and(|old| old != block)
    }

    /// Numbers of the slots whose block the conversation no longer owns.
    pub fn unowned(&self, owns: impl Fn(i64) -> bool) -> Vec<Idx> {
        self.fixed
            .iter()
            .filter_map(|slot| match &slot.kind {
                SlotKind::Block { id, .. } if !owns(*id) => Some(Idx::Fixed(slot.num)),
                _ => None,
            })
            .collect()
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

    /// Whether this session's own blocks have positions already.
    pub fn has_now(&self) -> bool {
        self.fixed.has_now()
    }

    /// The newest user message in hand, for the input box after a rewind.
    pub fn last_user_text(&self) -> Option<String> {
        self.fixed
            .iter()
            .filter_map(|slot| match &slot.kind {
                SlotKind::Block {
                    held: Some(held), ..
                } if held.block.kind == BlockKind::User => held
                    .block
                    .content
                    .clone()
                    .or_else(|| held.block.text.clone()),
                _ => None,
            })
            .last()
    }

    pub fn running(&self) -> &Mutable {
        &self.running
    }

    pub fn running_mut(&mut self) -> &mut Mutable {
        &mut self.running
    }

    /// Build the product this position hands over, once per change.
    pub fn prepare(&mut self, idx: Idx, opts: &Opts<'_>, width: usize) {
        match idx {
            Idx::Turn(at) => {
                let at = at as usize;
                if self.running.product(at).is_some() {
                    return;
                }
                let Some(product) = self.turn_product(at, opts, width) else {
                    return;
                };
                if !product.is_empty() {
                    self.running.set_product(at, product);
                }
            }
            Idx::Fixed(num) => {
                if self.fixed.product(num).is_some() {
                    return;
                }
                let Some(product) = self.fixed_product(num, opts, width) else {
                    return;
                };
                if !product.is_empty() {
                    self.fixed.set_product(num, product);
                }
            }
        }
    }

    /// What this position draws, as the run takes it.
    pub fn shape(&self, idx: Idx) -> Shape<'_> {
        match idx {
            Idx::Turn(at) => {
                let at = at as usize;
                let Some(entry) = self.running.get(at) else {
                    return Shape::Dead;
                };
                match entry {
                    Entry::Notice { text, level, .. } => {
                        Shape::Rows(product::notice_rows(text, *level))
                    }
                    Entry::Block(block) => match self.running.product(at) {
                        Some(Product::Text(text)) => {
                            Shape::Segs(text.segs(), block.kind == BlockKind::Reasoning)
                        }
                        Some(Product::Fixed(rows)) => Shape::Fixed(rows),
                        None => Shape::Dead,
                    },
                }
            }
            Idx::Fixed(num) => {
                let Some(slot) = self.fixed.at(num) else {
                    return Shape::Dead;
                };
                match &slot.kind {
                    SlotKind::Next => Shape::Dead,
                    SlotKind::Notice { text, level, .. } => {
                        Shape::Rows(product::notice_rows(text, *level))
                    }
                    SlotKind::Block { id, held } => match held {
                        None => Shape::Missing(*id),
                        Some(held) => match &held.product {
                            Some(Product::Text(text)) => {
                                Shape::Segs(text.segs(), held.block.kind == BlockKind::Reasoning)
                            }
                            Some(Product::Fixed(rows)) => Shape::Fixed(rows),
                            None => Shape::Dead,
                        },
                    },
                }
            }
        }
    }

    /// How many segments this position takes; `1` while its content is missing.
    pub fn count(&mut self, idx: Idx, opts: &Opts<'_>, width: usize) -> usize {
        self.prepare(idx, opts, width);
        match self.shape(idx) {
            Shape::Segs(segs, _) => segs.len(),
            Shape::Fixed(rows) => usize::from(!rows.is_empty()),
            Shape::Rows(rows) => usize::from(!rows.is_empty()),
            Shape::Missing(_) => 1,
            Shape::Dead => 0,
        }
    }

    /// The product this position handed over goes; the content stays.
    pub fn drop_product(&mut self, idx: Idx) {
        match idx {
            Idx::Fixed(num) => self.fixed.drop_product(num),
            Idx::Turn(at) => self.running.drop_product(at as usize),
        }
    }

    /// Every built product goes: the options or the width moved.
    pub fn drop_products(&mut self) {
        self.fixed.drop_products();
        self.running.drop_products();
    }

    /// Drop content past the region's budget; the returned numbers lost theirs.
    pub fn evict(&mut self, budget: usize, keep: impl Fn(i64, i64) -> bool) -> Vec<i64> {
        self.fixed.evict(budget, keep)
    }

    fn turn_product(&self, at: usize, opts: &Opts<'_>, width: usize) -> Option<Product> {
        let block = self.running.block(at)?;
        let paired = block
            .call_id
            .as_deref()
            .and_then(|call| self.running.result_of(call));
        Some(product::of(block, paired, opts, width))
    }

    fn fixed_product(&self, num: i64, opts: &Opts<'_>, width: usize) -> Option<Product> {
        let block = self.fixed.block(num)?;
        let paired = block
            .call_id
            .as_deref()
            .and_then(|call| self.fixed.pair(call))
            .and_then(|pair| pair.result)
            .and_then(|id| self.num_of(id))
            .and_then(|num| self.fixed.block(num));
        Some(product::of(block, paired, opts, width))
    }
}
