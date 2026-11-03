//! Display rows per block at one width: the layer that never survives a resize.

use std::collections::HashMap;

use super::super::line::Row;

/// One block's rows, and where each natural line starts.
pub(crate) struct Entry {
    pub(crate) rows: Vec<Row>,
    /// Row index of each natural line's first row.
    pub(crate) marks: Vec<usize>,
    /// Cells a natural line may use; zero for whole-block cards.
    pub(crate) avail: usize,
}

impl Entry {
    pub(crate) fn height(&self) -> usize {
        self.rows.len()
    }

    /// The row a natural line's offset falls on.
    pub(crate) fn row_of(&self, line: usize, off: usize) -> usize {
        let Some(start) = self.marks.get(line).copied() else {
            return self.rows.len().saturating_sub(1);
        };
        if self.avail == 0 {
            return start;
        }
        start + off / self.avail
    }

    /// The natural line a row belongs to, and the cells before it.
    pub(crate) fn spot_of(&self, row: usize) -> (usize, usize) {
        if self.marks.is_empty() || self.avail == 0 {
            return (0, 0);
        }
        let line = match self.marks.binary_search(&row) {
            Ok(at) => at,
            Err(at) => at.saturating_sub(1),
        };
        let start = self.marks.get(line).copied().unwrap_or(row);
        (line, (row - start) * self.avail)
    }
}

pub(crate) struct Rows {
    entries: HashMap<i64, Entry>,
    written: HashMap<i64, u64>,
    seq: u64,
    budget: usize,
}

impl Default for Rows {
    fn default() -> Self {
        Rows {
            entries: HashMap::new(),
            written: HashMap::new(),
            seq: 0,
            budget: 64,
        }
    }
}

impl Rows {
    pub(crate) fn set_budget(&mut self, budget: usize) {
        self.budget = budget.max(1);
    }

    pub(crate) fn entry(&self, id: i64) -> &Entry {
        self.entries.get(&id).unwrap_or(EMPTY_ENTRY)
    }

    pub(crate) fn insert(&mut self, id: i64, entry: Entry) {
        self.seq += 1;
        self.entries.insert(id, entry);
        self.written.insert(id, self.seq);
        self.evict();
    }

    pub(crate) fn known(&self, id: i64) -> Option<&Entry> {
        self.entries.get(&id)
    }

    pub(crate) fn drop_block(&mut self, id: i64) {
        self.entries.remove(&id);
        self.written.remove(&id);
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.written.clear();
        self.seq = 0;
    }

    fn evict(&mut self) {
        while self.entries.len() > self.budget {
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

static EMPTY_ENTRY: &Entry = &Entry {
    rows: Vec::new(),
    marks: Vec::new(),
    avail: 0,
};
