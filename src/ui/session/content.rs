//! The raw content cache: landed blocks by id, bounded, evicted by time then by id.

use std::collections::BTreeMap;

use super::event::Block;

/// The default number of blocks kept.
const DEFAULT_BUDGET: usize = 128;

/// Which side of the cache a read extends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Older,
    Newer,
}

/// The raw material: landed rows, and the budget they live under.
#[derive(Debug)]
pub struct ContentCache {
    budget: usize,
    clock: u64,
    blocks: BTreeMap<i64, (u64, Block)>,
}

impl Default for ContentCache {
    fn default() -> Self {
        ContentCache::new(DEFAULT_BUDGET)
    }
}

impl ContentCache {
    pub fn new(budget: usize) -> ContentCache {
        ContentCache {
            budget: budget.max(1),
            clock: 0,
            blocks: BTreeMap::new(),
        }
    }

    /// Set the budget; anything past it goes now.
    pub fn set_budget(&mut self, budget: usize) {
        self.budget = budget.max(1);
        self.evict();
    }

    /// Landed blocks, ascending by id.
    pub fn landed(&self) -> impl Iterator<Item = &Block> {
        self.blocks.values().map(|(_, block)| block)
    }

    /// Replace the whole cache.
    pub fn replace(&mut self, blocks: Vec<Block>) {
        self.blocks.clear();
        self.clock = 0;
        for block in blocks {
            self.insert(block);
        }
    }

    /// Merge landed rows; a row seen again takes a fresh admission time.
    pub fn merge(&mut self, blocks: Vec<Block>) {
        for block in blocks {
            self.insert(block);
        }
    }

    /// Drop the oldest admissions until the cache fits its budget.
    pub fn evict(&mut self) {
        while self.blocks.len() > self.budget {
            let victim = self
                .blocks
                .iter()
                .min_by_key(|(id, (stamp, _))| (*stamp, **id))
                .map(|(id, _)| *id);
            match victim {
                Some(id) => {
                    self.blocks.remove(&id);
                }
                None => break,
            }
        }
    }

    /// Drop every block the predicate rejects.
    pub fn retain(&mut self, keep: impl Fn(i64) -> bool) {
        self.blocks.retain(|id, _| keep(*id));
    }

    pub fn clear(&mut self) {
        self.blocks.clear();
        self.clock = 0;
    }

    fn insert(&mut self, block: Block) {
        self.clock += 1;
        self.blocks.insert(block.id, (self.clock, block));
    }
}
