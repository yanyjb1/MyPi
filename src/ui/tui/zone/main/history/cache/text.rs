//! Parsed lines per block: the layer that survives a width change.

use std::collections::HashMap;

use super::super::line::Row;

pub(crate) struct Text {
    blocks: HashMap<i64, Vec<Vec<Row>>>,
    written: HashMap<i64, u64>,
    seq: u64,
    budget: usize,
}

impl Default for Text {
    fn default() -> Self {
        Text {
            blocks: HashMap::new(),
            written: HashMap::new(),
            seq: 0,
            budget: 64,
        }
    }
}

impl Text {
    pub(crate) fn set_budget(&mut self, budget: usize) {
        self.budget = budget.max(1);
    }

    pub(crate) fn get(&mut self, id: i64, make: impl FnOnce() -> Vec<Vec<Row>>) -> &[Vec<Row>] {
        if !self.blocks.contains_key(&id) {
            self.seq += 1;
            self.blocks.insert(id, make());
            self.written.insert(id, self.seq);
            self.evict();
        }
        self.blocks.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub(crate) fn drop_block(&mut self, id: i64) {
        self.blocks.remove(&id);
        self.written.remove(&id);
    }

    pub(crate) fn clear(&mut self) {
        self.blocks.clear();
        self.written.clear();
        self.seq = 0;
    }

    fn evict(&mut self) {
        while self.blocks.len() > self.budget {
            let victim = self
                .written
                .iter()
                .min_by_key(|(_, stamp)| **stamp)
                .map(|(id, _)| *id);
            let Some(id) = victim else { break };
            self.drop_block(id);
        }
    }
}
