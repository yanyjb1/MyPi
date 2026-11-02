//! The content window: whole rows as they arrived, bounded, evicted by time then by id.

use std::collections::BTreeMap;

use serde_json::Value;

/// The default number of rows kept.
const DEFAULT_BUDGET: usize = 128;

/// The raw rows, each exactly as the answer carried it.
#[derive(Debug)]
pub struct ContentCache {
    budget: usize,
    clock: u64,
    rows: BTreeMap<i64, (u64, Value)>,
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
            rows: BTreeMap::new(),
        }
    }

    /// Set the budget; anything past it goes now.
    pub fn set_budget(&mut self, budget: usize) {
        self.budget = budget.max(1);
        self.evict();
    }

    /// Replace the whole window.
    pub fn replace(&mut self, rows: impl IntoIterator<Item = Value>) {
        self.rows.clear();
        self.clock = 0;
        for row in rows {
            self.insert(row);
        }
    }

    /// Merge rows; a row seen again takes a fresh admission time.
    pub fn merge(&mut self, rows: impl IntoIterator<Item = Value>) {
        for row in rows {
            self.insert(row);
        }
        self.evict();
    }

    /// Every id the ranges name is present.
    pub fn covers(&self, ranges: &[[i64; 2]]) -> bool {
        ranges.iter().all(|[lo, hi]| {
            let held = self.rows.range(*lo..=*hi).count() as i64;
            held == hi - lo + 1
        })
    }

    /// The rows of the ranges, in range order; the caller has checked [`covers`].
    pub fn select(&self, ranges: &[[i64; 2]]) -> Vec<Value> {
        let mut out = Vec::new();
        for [lo, hi] in ranges {
            out.extend(self.rows.range(*lo..=*hi).map(|(_, (_, row))| row.clone()));
        }
        out
    }

    /// Drop every row the predicate rejects.
    pub fn retain(&mut self, keep: impl Fn(i64) -> bool) {
        self.rows.retain(|id, _| keep(*id));
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.clock = 0;
    }

    fn insert(&mut self, row: Value) {
        let Some(id) = row.get("id").and_then(Value::as_i64) else {
            return;
        };
        self.clock += 1;
        self.rows.insert(id, (self.clock, row));
    }

    /// Drop the earliest admissions until the window fits its budget.
    fn evict(&mut self) {
        while self.rows.len() > self.budget {
            let victim = self
                .rows
                .iter()
                .min_by_key(|(id, (stamp, _))| (*stamp, **id))
                .map(|(id, _)| *id);
            match victim {
                Some(id) => {
                    self.rows.remove(&id);
                }
                None => break,
            }
        }
    }
}
