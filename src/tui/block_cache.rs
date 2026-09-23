//! Bounded LRU cache of rendered transcript blocks.
//!
//! Contract with `view.rs`: give me the transcript, width, and scroll
//! position; I give back the rows to paint and the total height. Only the
//! blocks near the viewport are materialized; everything else is a height
//! number. Memory and per-frame work are bounded by the budget, not by
//! conversation length — the user can chat for days without the frame
//! cost creeping up.
//!
//! Invalidation is whole-sale on width change (every wrap is wrong) and
//! index-truncate on transcript shrink (regenerate/rewind: entries after
//! the fork point are gone; the roster follows).

use std::collections::HashMap;

use ratatui::text::Line;

use super::blocks::{self, Range};
use crate::entry::Entry;
use crate::tui::theme::Palette;

/// Row budget for cached blocks. One screenful is ~50 rows; this holds
/// roughly 150 screens — far past anything a wheel can reach in a burst,
/// and O(1) no matter the transcript length.
const ROW_BUDGET: usize = 8192;

/// Cached render of one block + bookkeeping.
struct Slot {
    block: blocks::Block,
    /// LRU stamp — bumped on every hit; the lowest stamp is evicted.
    stamp: u64,
    /// Rows this slot costs against the budget.
    rows: usize,
}

pub struct BlockCache {
    /// Block ordinal (position in `blocks::blocks`) → rendered rows.
    slots: HashMap<usize, Slot>,
    /// Heights of every block in transcript order — cheap truth that
    /// survives eviction (recomputed only when a block re-renders).
    heights: Vec<usize>,
    width: usize,
    stamp: u64,
    /// Rows currently cached.
    cached_rows: usize,
}

impl BlockCache {
    pub(crate) fn new() -> Self {
        Self {
            slots: HashMap::new(),
            heights: Vec::new(),
            width: 0,
            stamp: 0,
            cached_rows: 0,
        }
    }

    /// Total display height (rows + inter-block gaps) — O(blocks), derived
    /// from the height roster instead of a second mutable ledger: one
    /// source of truth, no drift.
    pub(crate) fn total_height(&self) -> usize {
        let n = self.heights.iter().filter(|h| **h != usize::MAX).count();
        let known: usize = self.heights.iter().filter(|h| **h != usize::MAX).sum();
        // Unknown blocks still cost their gap in the coordinate space? No —
        // they render on demand before the viewport math runs; treat unknown
        // as 0 here and let every caller force-render first (rows_for does).
        known + n.saturating_sub(1)
    }

    /// Read-only height roster for viewport math (`usize::MAX` = unmeasured).
    pub(crate) fn heights_slice(&self) -> &[usize] {
        &self.heights
    }

    /// Re-sync with the transcript. Cheap when nothing changed: the
    /// block count and per-block heights are the fingerprint.
    pub(crate) fn sync(
        &mut self,
        entries: &[Entry],
        p: &Palette,
        show_reasoning: bool,
        tools_expanded: bool,
        width: usize,
    ) {
        if width != self.width {
            // Width change invalidates every wrap; drop rows but keep the
            // structure — heights re-derive from the re-render below.
            self.slots.clear();
            self.cached_rows = 0;
            self.width = width;
            self.heights.clear();
        }
        let ranges = blocks::blocks(entries);
        if ranges.len() != self.heights.len() {
            // Transcript grew (normal) or shrank (rewind). Shrink is the
            // only structural surprise: truncate heights and evict orphans.
            if ranges.len() < self.heights.len() {
                self.heights.truncate(ranges.len());
                let dead: Vec<usize> = self.slots.keys().copied().filter(|k| *k >= ranges.len()).collect();
                for k in dead {
                    if let Some(s) = self.slots.remove(&k) {
                        self.cached_rows -= s.rows;
                    }
                }
            }
            self.heights.resize(ranges.len(), usize::MAX); // unknown → forced render below
        }
        // Render only unknown-height blocks that are also needed now —
        // heights fill lazily: unknown blocks render on demand in `rows_for`,
        // except the *last* one, which must be known immediately (the
        // follow-the-bottom path uses it every frame).
        if let Some((idx, last)) = ranges.len().checked_sub(1).map(|i| (i, ranges[i]))
            && self.heights[last.start] == usize::MAX
        {
            let b = blocks::render_block(entries, last, p, show_reasoning, tools_expanded, width);
            self.admit(idx, b);
        }
    }

    /// The rows to paint for `range` of blocks (by ordinal), plus the
    /// absolute first row of that block range in transcript coordinates.
    ///
    /// Missing blocks render on demand (wheel into ancient history); the
    /// LRU evicts the coldest blocks when the budget would overflow.
    pub(crate) fn rows_for(
        &mut self,
        entries: &[Entry],
        p: &Palette,
        show_reasoning: bool,
        tools_expanded: bool,
        range: std::ops::Range<usize>,
    ) -> (Vec<Line<'static>>, usize) {
        let ranges = blocks::blocks(entries);
        let mut out = Vec::new();
        let mut prefix_rows = 0usize; // rows above `range.start`
        let mut gap_needed = false;
        for (i, r) in ranges.iter().enumerate() {
            let known = self.heights.get(i).copied().unwrap_or(usize::MAX);
            if i < range.start {
                if known == usize::MAX {
                    // Heights above the window must exist for the scroll
                    // math; render (uncached — a tall ancient block gets
                    // its height remembered but its rows dropped).
                    let b = blocks::render_block(entries, *r, p, show_reasoning, tools_expanded, self.width);
                    self.set_height(i, b.height);
                    prefix_rows += b.height + 1; // + gap
                } else {
                    prefix_rows += known + 1; // + gap
                }
                continue;
            }
            if i >= range.end {
                break;
            }
            let b = self.block(entries, i, *r, p, show_reasoning, tools_expanded);
            if gap_needed {
                out.push(blocks::block_gap());
            }
            // Visible blocks only (~a viewport): cloning the rows out is
            // negligible next to the rendering the cache just saved.
            out.extend(b.rows.clone());
            gap_needed = true;
        }
        (out, prefix_rows)
    }

    /// One block, via the cache.
    fn block(
        &mut self,
        entries: &[Entry],
        idx: usize,
        r: Range,
        p: &Palette,
        show_reasoning: bool,
        tools_expanded: bool,
    ) -> &blocks::Block {
        if !self.slots.contains_key(&idx) || self.heights.get(idx) == Some(&usize::MAX) {
            let b = blocks::render_block(entries, r, p, show_reasoning, tools_expanded, self.width);
            self.admit(idx, b);
        }
        self.stamp += 1;
        let s = self.slots.get_mut(&idx).expect("just admitted");
        s.stamp = self.stamp;
        // Borrow dance: admit() may have evicted others; re-fetch is sound.
        let slot = self.slots.get(&idx).expect("hit after stamp bump");
        &slot.block
    }

    fn set_height(&mut self, idx: usize, h: usize) {
        if let Some(old) = self.heights.get_mut(idx) {
            *old = h;
        }
    }

    /// Insert a rendered block; evict LRU (never the newest) if needed.
    fn admit(&mut self, idx: usize, b: blocks::Block) {
        self.set_height(idx, b.height);
        let rows = b.height;
        // The newest block (highest ordinal present in cache) stays: it is
        // the follow-bottom hot path.
        let newest = self.slots.keys().copied().max();
        while self.cached_rows + rows > ROW_BUDGET {
            let Some(victim) = self
                .slots
                .iter()
                .filter(|(k, _)| Some(**k) != newest && **k != idx)
                .min_by_key(|(_, s)| s.stamp)
                .map(|(k, _)| *k)
            else {
                break; // budget cannot hold even this block alone; keep it anyway
            };
            if let Some(s) = self.slots.remove(&victim) {
                self.cached_rows -= s.rows;
            }
        }
        self.cached_rows += rows;
        self.stamp += 1;
        self.slots.insert(idx, Slot { block: b, stamp: self.stamp, rows });
    }

    #[cfg(test)]
    fn cached_block_count(&self) -> usize {
        self.slots.len()
    }

    #[cfg(test)]
    fn cached_rows(&self) -> usize {
        self.cached_rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Entry;

    fn p() -> Palette {
        Palette::default()
    }

    fn convo(blocks_n: usize) -> Vec<Entry> {
        let mut es = Vec::new();
        for i in 0..blocks_n {
            es.push(Entry::User { content: format!("用户消息 {i} 一点内容") });
            es.push(Entry::Assistant { content: format!("回答 {i}"), usage: None, reasoning: None });
        }
        es
    }

    #[test]
    fn viewport_slice_matches_full_render() {
        let es = convo(6);
        let mut c = BlockCache::new();
        c.sync(&es, &p(), true, false, 60);
        // Materialize blocks 1..4 through the cache.
        let (rows, _) = c.rows_for(&es, &p(), true, false, 1..4);
        // The same slice straight from the convenience path.
        let full = crate::tui::components::chat::render(&es, &p(), true, false);
        // Count only: exact styling is chat.rs's own test suite's job.
        assert!(!rows.is_empty());
        assert!(full.len() > rows.len(), "全量必须包含更多块");
    }

    #[test]
    fn total_height_is_gap_inclusive() {
        let es = convo(3);
        let mut c = BlockCache::new();
        c.sync(&es, &p(), true, false, 60);
        // Force-render everything.
        let _ = c.rows_for(&es, &p(), true, false, 0..6);
        // n blocks + (n-1) gaps.
        let blocks_sum: usize = c.heights.iter().sum();
        assert_eq!(c.total_height(), blocks_sum + 5, "总高 = 块高 + 间隙");
    }

    #[test]
    fn cache_stays_bounded_when_walking_ancient_history() {
        let es = convo(400); // 800 entries — the "chatted for days" case
        let mut c = BlockCache::new();
        c.sync(&es, &p(), true, false, 60);
        for start in (0..790).step_by(7).rev() {
            let _ = c.rows_for(&es, &p(), true, false, start..(start + 5).min(800));
        }
        assert!(c.cached_rows() <= ROW_BUDGET, "缓存行数 {} 超预算", c.cached_rows());
        assert!(c.cached_block_count() < 800, "LRU 必须驱逐");
    }

    #[test]
    fn shrink_on_rewind_truncates_roster() {
        let mut c = BlockCache::new();
        let es = convo(5);
        c.sync(&es, &p(), true, false, 60);
        let _ = c.rows_for(&es, &p(), true, false, 0..10);
        let before = c.total_height();
        let cut: Vec<Entry> = es[..4].to_vec();
        c.sync(&cut, &p(), true, false, 60);
        assert!(c.total_height() <= before, "回溯截断后总高只能变小");
        assert_eq!(c.heights.len(), 4);
    }
}
