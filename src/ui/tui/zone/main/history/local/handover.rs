//! The handover: the running turn's blocks take their numbers.

use super::section::Local;

impl Local {
    /// The round's structure arrived: give its blocks numbers. `true` = handed over.
    pub(crate) fn handover(&mut self, ranges: &[[i64; 2]]) -> bool {
        let mut ids: Vec<i64> = Vec::new();
        for [lo, hi] in ranges {
            ids.extend(*lo..=*hi);
        }
        if ids.is_empty() || ids.len() != self.live().len() {
            return false;
        }
        let blocks = self.live_mut().take();
        self.kill_short_lived();
        for (index, (id, mut block)) in ids.into_iter().zip(blocks).enumerate() {
            block.id = id;
            block.first = index == 0;
            self.append(block);
        }
        true
    }
}
