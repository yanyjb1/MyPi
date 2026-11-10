//! The handover: the running turn's blocks take their numbers.
//!
//! A notice never takes an id: it is counted out first, then put between the blocks in order.

use super::mutable::Entry;
use super::section::Local;

impl Local {
    /// The round's receipt arrived and lines up with the running turn: blocks take ids, notices
    /// keep their place. `false` = it does not line up; the receipt numbers its own rows then.
    pub(crate) fn handover(&mut self, ranges: &[[i64; 2]]) -> bool {
        let ids = ids_of(ranges);
        let blocks = self
            .running()
            .entries()
            .iter()
            .filter(|entry| matches!(entry, Entry::Block(_)))
            .count();
        if ids.is_empty() || ids.len() != blocks {
            return false;
        }
        let entries = self.running_mut().take();
        let _ = self.kill_short();
        let mut ids = ids.into_iter();
        let mut first = true;
        for entry in entries {
            match entry {
                Entry::Block(mut block) => {
                    let Some(id) = ids.next() else {
                        return false;
                    };
                    block.id = id;
                    block.first = first;
                    first = false;
                    self.append(block);
                }
                Entry::Notice { text, level, life } => {
                    self.append_notice(text, level, life);
                }
            }
        }
        true
    }

    /// The receipt named nothing: nothing landed, so the turn's blocks go and its notices stay.
    pub(crate) fn drop_turn(&mut self) {
        let _ = self.kill_short();
        for entry in self.running_mut().take() {
            if let Entry::Notice { text, level, life } = entry {
                self.append_notice(text, level, life);
            }
        }
    }
}

/// Every id the receipt names, in order.
fn ids_of(ranges: &[[i64; 2]]) -> Vec<i64> {
    let mut ids: Vec<i64> = Vec::new();
    for [lo, hi] in ranges {
        ids.extend(*lo..=*hi);
    }
    ids
}
