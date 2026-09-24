//! Bounded LRU cache of rendered transcript **blocks**.
//!
//! The cache unit is the block (one user message / assistant turn / tool
//! exchange / system notice), not the row. Each slot stores the rendered
//! rows for the block **in both fold states** (tool outputs render folded
//! per their threshold and expanded; every other block renders the same
//! rows for both). Row height is a *derived* property of the rendered
//! rows — nothing is measured ahead of time, nothing is rendered "just to
//! measure and thrown away". Blocks the viewport never touches are never
//! rendered at all.
//!
//! Scrolling: `view.rs` walks **up from the bottom** (`chat_scroll` is
//! already defined as rows-up-from-bottom). Unknown heights are measured
//! on demand during that walk, so cold start renders exactly one
//! viewport. Walking into never-rendered ancient history pays a one-time
//! measure-as-you-go cost that then stays cached.
//!
//! Eviction: by **block count** (`BLOCK_BUDGET`, 256). Blocks average
//! ~14 rows, so 256 blocks ≈ 3.7k rows ≈ 1 MB of spans — comfortably
//! under the old 8192-row budget, with a hard worst case (all-huge
//! blocks) around 7 MB. Memory and per-frame work stay bounded by the
//! budget, not the conversation length.
//!
//! Invalidation is whole-sale on width/epoch change (every wrap is wrong)
//! and index-truncate on transcript shrink (regenerate/rewind: entries
//! after the fork point are gone).

use std::collections::HashMap;

use ratatui::text::Line;

use super::blocks::{self, Range};
use crate::entry::Entry;
use crate::tui::theme::Palette;

/// Block-count budget for cached blocks. Blocks average ~14 rows (24k-entry
/// bench), so this is ≈3.7k rows ≈ 1 MB of span data; the pathological
/// all-huge-blocks worst case is ~7 MB. Roughly 6 screens cached per wheel
/// page — far past any wheel burst, and O(1) in transcript length.
const BLOCK_BUDGET: usize = 256;

/// Cached render of one block in both fold states + bookkeeping.
struct Slot {
    /// Rows when tools are folded (the default view).
    folded: blocks::Block,
    /// Rows when tools are expanded (Ctrl+O). For blocks with no foldable
    /// content this is the same allocation as `folded` (Arc-shared is
    /// overkill; equal-by-construction blocks just render twice at admit
    /// time and cost one extra Vec — negligible vs the render itself).
    expanded: blocks::Block,
    /// LRU stamp — bumped on every hit; the lowest stamp is evicted.
    stamp: u64,
}

pub struct BlockCache {
    /// Block ordinal (position in `blocks::blocks`) → rendered rows.
    slots: HashMap<usize, Slot>,
    /// Heights of every measured block, transcript order. `usize::MAX` =
    /// never rendered (never measured). Heights are derived facts, cheap
    /// to keep for all blocks (8 bytes each) and they survive eviction —
    /// scroll math over visited history stays O(1) forever.
    heights: Vec<usize>,
    width: usize,
    stamp: u64,
    /// Theme epoch the cached rows were colored with. A theme swap bumps
    /// the epoch; the next sync drops everything (heights re-derive from
    /// the re-render). omp's `themeEpoch` cache-key contract.
    theme_epoch: u64,
}

impl BlockCache {
    /// Bench/preview harness constructor (doc-hidden, not API).
    #[doc(hidden)]
    pub fn new_public() -> Self {
        Self::new()
    }

    /// Bench harness getters (doc-hidden, not API).
    #[doc(hidden)]
    pub fn cached_rows(&self) -> usize {
        self.slots
            .values()
            .map(|s| s.folded.height + s.expanded.height)
            .sum()
    }

    #[doc(hidden)]
    pub fn cached_blocks(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn new() -> Self {
        Self {
            slots: HashMap::new(),
            heights: Vec::new(),
            width: 0,
            stamp: 0,
            theme_epoch: crate::tui::theme::theme_epoch(),
        }
    }

    /// Total display height over **measured** blocks (rows + gaps).
    /// Unmeasured blocks count as gap-only. Debug/test/diagnostic surface:
    /// the reverse scroll walk does not need it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn total_height(&self) -> usize {
        let n = self.heights.iter().filter(|h| **h != usize::MAX).count();
        let known: usize = self.heights.iter().filter(|h| **h != usize::MAX).sum();
        known + n.saturating_sub(1)
    }

    /// Re-sync with the transcript. Cheap when nothing changed: the
    /// block count and per-block heights are the fingerprint.
    pub(crate) fn sync(
        &mut self,
        entries: &[Entry],
        _p: &Palette,
        _show_reasoning: bool,
        _tools_expanded: bool,
        width: usize,
    ) {
        let epoch = crate::tui::theme::theme_epoch();
        if epoch != self.theme_epoch {
            self.slots.clear();
            self.heights.clear();
            self.theme_epoch = epoch;
        }
        if width != self.width {
            // Width change invalidates every wrap; drop rows but keep the
            // structure — heights re-derive from the re-render below.
            self.slots.clear();
            self.width = width;
            self.heights.clear();
        }
        let ranges = blocks::blocks(entries);
        if ranges.len() != self.heights.len() {
            // Transcript grew (normal) or shrank (rewind). Shrink is the
            // only structural surprise: truncate heights and evict orphans.
            if ranges.len() < self.heights.len() {
                self.heights.truncate(ranges.len());
                let dead: Vec<usize> = self
                    .slots
                    .keys()
                    .copied()
                    .filter(|k| *k >= ranges.len())
                    .collect();
                for k in dead {
                    self.slots.remove(&k);
                }
            }
            self.heights.resize(ranges.len(), usize::MAX); // unknown → measure on demand
        }
    }

    /// Measure one block's height without caching its rows? No such thing
    /// anymore — measuring IS rendering. Height arrives with the render.
    fn render_both(
        &self,
        entries: &[Entry],
        r: Range,
        p: &Palette,
        show_reasoning: bool,
        width: usize,
    ) -> (blocks::Block, blocks::Block) {
        let folded = blocks::render_block(entries, r, p, show_reasoning, false, width);
        let expanded = blocks::render_block(entries, r, p, show_reasoning, true, width);
        (folded, expanded)
    }

    /// The rows to paint for `range` of blocks (by ordinal), plus the
    /// number of **known** rows above `range.start` (for the splice in
    /// view.rs). Unmeasured blocks inside the range render now; unmeasured
    /// blocks above the range do **not** render — the caller's scroll walk
    /// (reverse, from the bottom) measures those it actually crosses.
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
        let mut prefix_rows = 0usize; // known rows above `range.start`
        let mut gap_needed = false;
        for (i, r) in ranges.iter().enumerate() {
            let known = self.heights.get(i).copied().unwrap_or(usize::MAX);
            if i < range.start {
                // Only *known* heights count toward the splice prefix.
                // Unknown ones belong to never-visited history; the scroll
                // walk measures them before this window is computed.
                if known != usize::MAX {
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
            out.extend(b.rows.clone());
            gap_needed = true;
        }
        (out, prefix_rows)
    }

    /// Reverse scroll walk: given the desired rows-up-from-bottom offset,
    /// find the block range whose rows cover `[offset, offset+viewport)`.
    ///
    /// Walks from the last block backwards, measuring unmeasured blocks as
    /// it goes (measure = render; the rows stay cached and immediately
    /// useful). Stops as soon as enough height has accumulated. This is
    /// the whole point of the block cache: nothing above the walk's stop
    /// point is ever touched, so cold start renders one viewport.
    pub(crate) fn window_from_bottom(
        &mut self,
        entries: &[Entry],
        p: &Palette,
        show_reasoning: bool,
        offset_rows: usize,
        viewport_rows: usize,
    ) -> (usize, usize) {
        let ranges = blocks::blocks(entries);
        let n = ranges.len();
        if n == 0 {
            return (0, 0);
        }
        // Accumulate rows upward: we need `offset_rows + viewport_rows`
        // rows below the window's top edge.
        let need = offset_rows.saturating_add(viewport_rows);
        let mut acc = 0usize;
        let b1 = n; // exclusive end
        let mut b0 = n;
        for i in (0..n).rev() {
            let h = match self.heights.get(i).copied().unwrap_or(usize::MAX) {
                usize::MAX => {
                    // Measure = render: both fold states render once and
                    // land in the cache; this block is inside the requested
                    // span, so the rows are immediately useful.
                    let (f, e) =
                        self.render_both(entries, ranges[i], p, show_reasoning, self.width);
                    let h = f.height;
                    self.admit(i, f, e);
                    h
                }
                h => h,
            };
            acc += h + 1; // + gap
            b0 = i;
            if acc >= need {
                break;
            }
        }
        // Trim the top: blocks entirely above the window's top edge drop
        // out of the window (their height already counted in the walk).
        // One block of slack above for smooth wheeling.
        (b0.saturating_sub(1), b1.min(n))
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
            let (f, e) = self.render_both(entries, r, p, show_reasoning, self.width);
            self.admit(idx, f, e);
        }
        self.stamp += 1;
        let s = self.slots.get_mut(&idx).expect("just admitted");
        s.stamp = self.stamp;
        // Borrow dance: admit() may have evicted others; re-fetch is sound.
        let slot = self.slots.get(&idx).expect("hit after stamp bump");
        if tools_expanded {
            &slot.expanded
        } else {
            &slot.folded
        }
    }

    /// Insert a rendered block (both fold states); evict LRU (never the
    /// newest) when the block budget would overflow.
    fn admit(&mut self, idx: usize, folded: blocks::Block, expanded: blocks::Block) {
        let h = folded.height;
        self.heights[idx] = h;
        // The newest block (highest ordinal present in cache) stays: it is
        // the follow-bottom hot path.
        let newest = self.slots.keys().copied().max();
        while self.slots.len() >= BLOCK_BUDGET {
            let Some(victim) = self
                .slots
                .keys()
                .copied()
                .filter(|k| Some(*k) != newest && *k != idx)
                .min_by_key(|k| self.slots[k].stamp)
            else {
                break; // budget full of pinned blocks; keep anyway
            };
            self.slots.remove(&victim);
        }
        self.stamp += 1;
        self.slots.insert(
            idx,
            Slot {
                folded,
                expanded,
                stamp: self.stamp,
            },
        );
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
            es.push(Entry::User {
                content: format!("用户消息 {i} 一点内容"),
            });
            es.push(Entry::Assistant {
                content: format!("回答 {i}"),
                usage: None,
                reasoning: None,
            });
        }
        es
    }

    #[test]
    fn cold_start_renders_only_the_bottom_window() {
        // The contract: a huge transcript + a bottom-window request must
        // NOT render the whole thing just to measure heights.
        let es = convo(2000); // 4000 entries
        let mut c = BlockCache::new();
        c.sync(&es, &p(), true, false, 60);
        let n = blocks::blocks(&es).len();
        let (b0, b1) = c.window_from_bottom(&es, &p(), true, 0, 40);
        let _ = c.rows_for(&es, &p(), true, false, b0..b1);
        assert!(
            c.cached_blocks() < n / 4,
            "冷启动必须只渲染底部窗口：{} / {} 块",
            c.cached_blocks(),
            n
        );
    }

    #[test]
    fn window_from_bottom_covers_the_requested_span() {
        let es = convo(30);
        let mut c = BlockCache::new();
        c.sync(&es, &p(), true, false, 60);
        // Ask for the bottom 40 rows.
        let (b0, b1) = c.window_from_bottom(&es, &p(), true, 0, 40);
        assert_eq!(b1, blocks::blocks(&es).len());
        let (rows, _) = c.rows_for(&es, &p(), true, false, b0..b1);
        assert!(!rows.is_empty());
    }

    #[test]
    fn cache_stays_bounded_when_walking_ancient_history() {
        let es = convo(400); // 800 entries — the "chatted for days" case
        let mut c = BlockCache::new();
        c.sync(&es, &p(), true, false, 60);
        // Walk to the top through windows, like a wheel burst would.
        let n = blocks::blocks(&es).len();
        let mut top = n;
        while top > 0 {
            let b0 = top.saturating_sub(8);
            let _ = c.rows_for(&es, &p(), true, false, b0..top);
            top = b0;
        }
        assert!(
            c.cached_blocks() <= BLOCK_BUDGET,
            "LRU 必须按块数封顶：{}",
            c.cached_blocks()
        );
    }

    #[test]
    fn heights_survive_eviction() {
        // Evicted rows leave their heights behind: scroll math over visited
        // history stays exact even after the rows are gone.
        let es = convo(400);
        let mut c = BlockCache::new();
        c.sync(&es, &p(), true, false, 60);
        let n = blocks::blocks(&es).len();
        let _ = c.rows_for(&es, &p(), true, false, 0..8.min(n));
        for i in 0..8.min(n) {
            assert_ne!(c.heights[i], usize::MAX, "height {i} must persist");
        }
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
