//! The handover: the running turn's blocks take their numbers.
//!
//! A notice never takes an id: it is counted out first, then put between the blocks in order.

use super::mutable::Entry;
use super::section::Local;

impl Local {
    /// The round's receipt arrived: blocks take ids, notices keep their place. `true` = handed over.
    pub(crate) fn handover(&mut self, ranges: &[[i64; 2]]) -> bool {
        let mut ids: Vec<i64> = Vec::new();
        for [lo, hi] in ranges {
            ids.extend(*lo..=*hi);
        }
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
        self.kill_short();
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
}
