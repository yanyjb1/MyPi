//! The rows painted for each slot, and the keys they were built under.

use std::collections::HashMap;

use ratatui::text::Line;

use crate::ui::tui::theme::theme_epoch;

/// How many slots keep their rows around the sight line.
pub(in crate::ui::tui::zone::main::history) const BUDGET: usize = 65;

pub(in crate::ui::tui::zone::main::history) struct BlockCache {
    rows: HashMap<i64, Vec<Line<'static>>>,
    /// Admission order, oldest first.
    written: HashMap<i64, u64>,
    seq: u64,
    width: usize,
    cwd: String,
    epoch: u64,
    switches: (bool, bool),
    budget: usize,
}

impl Default for BlockCache {
    fn default() -> Self {
        Self {
            rows: HashMap::new(),
            written: HashMap::new(),
            seq: 0,
            width: 0,
            cwd: String::new(),
            epoch: u64::MAX,
            switches: (false, false),
            budget: BUDGET,
        }
    }
}

impl BlockCache {
    pub(in crate::ui::tui::zone::main::history) fn set_budget(&mut self, budget: usize) {
        self.budget = budget.max(1);
    }

    /// Drop everything built under keys that no longer hold.
    pub(in crate::ui::tui::zone::main::history) fn sync(
        &mut self,
        width: usize,
        cwd: &str,
        switches: (bool, bool),
    ) {
        let epoch = theme_epoch();
        if epoch != self.epoch
            || width != self.width
            || cwd != self.cwd
            || switches != self.switches
        {
            self.clear();
            self.epoch = epoch;
            self.width = width;
            self.cwd.clear();
            self.cwd.push_str(cwd);
            self.switches = switches;
        }
    }

    /// This slot's rows, painted now if they are not here.
    pub(in crate::ui::tui::zone::main::history) fn paint(
        &mut self,
        num: i64,
        make: impl FnOnce() -> Vec<Line<'static>>,
    ) -> &[Line<'static>] {
        if !self.rows.contains_key(&num) {
            let rows = make();
            self.seq += 1;
            self.rows.insert(num, rows);
            self.written.insert(num, self.seq);
            self.evict();
        }
        self.rows.get(&num).map(Vec::as_slice).unwrap_or(&[])
    }

    /// This slot's height, if its rows are here.
    pub(in crate::ui::tui::zone::main::history) fn known(&self, num: i64) -> Option<usize> {
        self.rows.get(&num).map(Vec::len)
    }

    pub(in crate::ui::tui::zone::main::history) fn drop_num(&mut self, num: i64) {
        self.rows.remove(&num);
        self.written.remove(&num);
    }

    pub(in crate::ui::tui::zone::main::history) fn clear(&mut self) {
        self.rows.clear();
        self.written.clear();
        self.seq = 0;
    }

    /// Keep at most `budget` slots: the earliest admission goes first.
    fn evict(&mut self) {
        while self.rows.len() > self.budget {
            let victim = self
                .written
                .iter()
                .min_by_key(|(_, stamp)| **stamp)
                .map(|(num, _)| *num);
            match victim {
                Some(num) => {
                    self.rows.remove(&num);
                    self.written.remove(&num);
                }
                None => break,
            }
        }
    }
}
