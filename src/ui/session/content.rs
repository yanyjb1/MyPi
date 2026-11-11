//! The content window: whole blocks as they arrived, bounded, evicted by time then by id.

use std::collections::BTreeMap;

use serde_json::Value;

/// The default number of blocks kept.
const DEFAULT_BUDGET: usize = 128;

/// The blocks, each whole, exactly as the answer carried it.
///
/// One entry is one block: the key is its id, and the value is the whole block plus the admission
/// stamp that orders it. A value without an id is not a block and never enters.
#[derive(Debug)]
pub struct ContentCache {
    budget: usize,
    clock: u64,
    blocks: BTreeMap<i64, (u64, Value)>,
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

    /// Replace the whole window: the blocks an answer carried, cut to the budget.
    pub fn replace(&mut self, blocks: impl IntoIterator<Item = Value>) {
        self.blocks.clear();
        self.clock = 0;
        for block in blocks {
            self.insert(block);
        }
        self.evict();
    }

    /// Merge blocks; a block seen again takes a fresh admission time.
    pub fn merge(&mut self, blocks: impl IntoIterator<Item = Value>) {
        for block in blocks {
            self.insert(block);
        }
        self.evict();
    }

    /// Every block the ranges name is here.
    pub fn covers(&self, ranges: &[[i64; 2]]) -> bool {
        ranges.iter().all(|[lo, hi]| {
            let held = self.blocks.range(*lo..=*hi).count() as i64;
            held == hi - lo + 1
        })
    }

    /// The blocks of the ranges, in range order; the caller has checked [`covers`].
    pub fn select(&self, ranges: &[[i64; 2]]) -> Vec<Value> {
        let mut out = Vec::new();
        for [lo, hi] in ranges {
            out.extend(
                self.blocks
                    .range(*lo..=*hi)
                    .map(|(_, (_, block))| block.clone()),
            );
        }
        out
    }

    /// Drop every block the predicate rejects.
    pub fn retain(&mut self, keep: impl Fn(i64) -> bool) {
        self.blocks.retain(|id, _| keep(*id));
    }

    pub fn clear(&mut self) {
        self.blocks.clear();
        self.clock = 0;
    }

    fn insert(&mut self, block: Value) {
        let Some(id) = block.get("id").and_then(Value::as_i64) else {
            return;
        };
        self.clock += 1;
        self.blocks.insert(id, (self.clock, block));
    }

    /// Drop the earliest admissions until the window fits its budget.
    fn evict(&mut self) {
        let over = self.blocks.len().saturating_sub(self.budget);
        if over == 0 {
            return;
        }
        let mut order: Vec<(u64, i64)> = self
            .blocks
            .iter()
            .map(|(id, (stamp, _))| (*stamp, *id))
            .collect();
        order.sort_unstable();
        for (_, id) in order.into_iter().take(over) {
            self.blocks.remove(&id);
        }
    }
}
