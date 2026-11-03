//! The handover: the growing tail's blocks become numbered slots of the finished region.

use super::local::SlotKind;
use super::zone::HistoryZone;

impl HistoryZone {
    /// The round's structure arrived: give the tail its numbers. `true` = the tail was handed over.
    pub(super) fn handover(&mut self, ranges: &[[i64; 2]]) -> bool {
        let mut ids: Vec<i64> = Vec::new();
        for [lo, hi] in ranges {
            ids.extend(*lo..=*hi);
        }
        if self.needs_layout || ids.is_empty() || ids.len() != self.live.len() {
            return false;
        }
        let blocks = self.live.take();
        self.local.kill_short_lived();
        for (index, (id, mut block)) in ids.into_iter().zip(blocks).enumerate() {
            block.id = id;
            block.first = index == 0;
            self.blocks.insert(id, block);
            self.landed = self.landed.max(id);
            self.local.push_now(SlotKind::Block(id));
        }
        true
    }
}
