//! Bounded LRU cache of rendered transcript **blocks**.
//!
//! The cache unit is the block (one user message / assistant turn / tool
//! exchange / system notice), not the row. Each slot stores the rendered
//! rows for every **variant** the block actually has: a block reacts to at
//! most one view switch (tool exchanges to Ctrl+O's `tools_expanded`),
//! and only when its
//! content actually changes between the switch's two positions (a short
//! tool output renders identically folded and expanded — one variant, one
//! render, one copy). Row height is a *derived* property of the rendered
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
//!
//! Heights are per-variant (`[usize; 2]`): slot 0 = the switch OFF, slot
//! 1 = the switch ON, single-variant blocks occupy both with the same
//! value (8 wasted bytes per block buys uniform indexing). `usize::MAX`
//! = never measured.

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

/// Cached render of one block's **variants** + bookkeeping.
///
/// `variants` holds 1 or 2 rendered forms: two only when the block's
/// content actually differs across its view switch (foldable tool output,
/// non-empty reasoning). Single-variant blocks render and store once.
struct Slot {
    /// Variant 0 = switch OFF, variant 1 = switch ON (`len` 1 or 2).
    variants: Vec<blocks::Block>,
    /// LRU stamp — bumped on every hit; the lowest stamp is evicted.
    stamp: u64,
}

impl Slot {
    /// The variant at `index` (0 = switch off, 1 = switch on). Panics on
    /// an out-of-range index — callers derive it from `variants.len()`.
    fn pick(&self, index: usize) -> &blocks::Block {
        &self.variants[index.min(self.variants.len() - 1)]
    }
}

/// The view switch a block reacts to, if any. Derived from the block's
/// first entry; a block is sensitive to **at most one** switch (assistant
/// blocks carry the reasoning, tool exchanges the fold).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Switch {
    None,
    /// Ctrl+T (`show_reasoning`).
    Reasoning,
    /// Ctrl+O (`tools_expanded`).
    Tools,
}

/// Which switch changes this block's rendering. The single place that
/// maps block kind → sensitive switch.
fn block_switch(group: &[Entry]) -> Switch {
    match group.first() {
        Some(Entry::Reasoning { .. }) => Switch::Reasoning,
        Some(Entry::ToolRequest { .. }) if group.len() == 2 => Switch::Tools,
        _ => Switch::None,
    }
}

pub struct BlockCache {
    /// Block ordinal (position in `blocks::blocks`) → rendered rows.
    slots: HashMap<usize, Slot>,
    /// Per-variant heights of every measured block, transcript order.
    /// Index 0 = switch OFF, 1 = switch ON (single-variant blocks keep
    /// both equal). `usize::MAX` = never rendered (never measured).
    /// Heights are derived facts, cheap to keep for all blocks (16 bytes
    /// each) and they survive eviction — scroll math over visited history
    /// stays O(1) forever.
    heights: Vec<[usize; 2]>,
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
            .map(|s| s.variants.iter().map(|b| b.height).sum::<usize>())
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

    /// Total display height over **measured** blocks (rows + gaps),
    /// counting variant 0 (switch OFF). Unmeasured blocks count as
    /// gap-only. Debug/test/diagnostic surface: the reverse scroll walk
    /// does not need it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn total_height(&self) -> usize {
        let known: usize = self
            .heights
            .iter()
            .filter(|h| h[0] != usize::MAX)
            .map(|h| h[0])
            .sum();
        let n = self.heights.iter().filter(|h| h[0] != usize::MAX).count();
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
            self.heights.resize(ranges.len(), [usize::MAX; 2]); // unknown → measure on demand
        }
    }

    /// Render the block's **actual** variants (1 or 2). Returns the
    /// rendered forms in order: `[switch-off]` or `[switch-off,
    /// switch-on]` — empty never happens (a block always renders).
    ///
    /// A block reacts to at most one switch: tool exchanges to
    /// `exchange_has_two_states` is the truth). A reasoning block's
    /// "two states" are *visible vs hidden* — but hidden renders zero
    /// rows and we do not cache emptiness, so it is single-variant too.
    /// Everything else is single-variant, rendered exactly once.
    fn render_variants(
        &self,
        entries: &[Entry],
        r: Range,
        p: &Palette,
        show_reasoning: bool,
        tools_expanded: bool,
        width: usize,
    ) -> Vec<blocks::Block> {
        let group = &entries[r.start..r.end];
        // The (show_reasoning, tools_expanded) pairs to render for the
        // switch-off and switch-on positions of this block's sensitive
        // switch, plus whether the block actually has two forms.
        let (off, on, two) = match &group[0] {
            // Reasoning blocks are single-variant: Ctrl+T hides the block
            // outright (view.rs skips it), so the cache only ever stores
            // the visible form.
            Entry::Reasoning { .. } => ((true, tools_expanded), (true, tools_expanded), false),
            Entry::ToolRequest { name, .. } if group.len() == 2 => {
                let Entry::ToolResult { ok, result, .. } = &group[1] else {
                    unreachable!("group of 2 is always request+result (blocks guarantees)");
                };
                let two = super::components::cards::exchange_has_two_states(name, *ok, result);
                ((show_reasoning, false), (show_reasoning, true), two)
            }
            _ => (
                (show_reasoning, tools_expanded),
                (show_reasoning, tools_expanded),
                false,
            ),
        };
        let mut out = Vec::with_capacity(if two { 2 } else { 1 });
        out.push(blocks::render_block(entries, r, p, off.0, off.1, width));
        if two {
            out.push(blocks::render_block(entries, r, p, on.0, on.1, width));
        }
        out
    }

    /// Which cached variant answers the current view state: 0 = switch
    /// off, 1 = switch on. Single-variant slots always answer 0.
    fn variant_index(slot: &Slot, switch: Switch, view: (bool, bool)) -> usize {
        if slot.variants.len() == 1 {
            return 0;
        }
        match switch {
            Switch::None => 0,
            Switch::Reasoning => usize::from(view.0),
            Switch::Tools => usize::from(view.1),
        }
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
            let known = self.heights.get(i);
            if i < range.start {
                // Only *known* heights count toward the splice prefix.
                // Unknown ones belong to never-visited history; the scroll
                // walk measures them before this window is computed.
                if let Some(h) = known
                    && h[0] != usize::MAX
                {
                    prefix_rows += h[0] + 1; // + gap
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
        tools_expanded: bool,
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
        let mut b0 = n;
        let b1 = n; // exclusive end
        for i in (0..n).rev() {
            // The variant this walk measures under the current view
            // state. Single-variant blocks keep both height slots equal,
            // so indexing [0/1] by the switch is safe even when the
            // block's rows were evicted (heights outlive slots).
            let switch = block_switch(&entries[ranges[i].start..ranges[i].end]);
            let slot_index = match switch {
                Switch::None => 0,
                Switch::Reasoning => usize::from(show_reasoning),
                Switch::Tools => usize::from(tools_expanded),
            };
            let h = match self.heights.get(i) {
                None | Some([usize::MAX, _]) => {
                    // Measure = render: the block's variants render once
                    // and land in the cache; this block is inside the
                    // requested span, so the rows are immediately useful.
                    let variants = self.render_variants(
                        entries,
                        ranges[i],
                        p,
                        show_reasoning,
                        tools_expanded,
                        self.width,
                    );
                    self.admit(i, variants);
                    self.heights[i][slot_index]
                }
                Some(h) => h[slot_index],
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

    /// One block, via the cache, in the variant the current view state
    /// asks for.
    fn block(
        &mut self,
        entries: &[Entry],
        idx: usize,
        r: Range,
        p: &Palette,
        show_reasoning: bool,
        tools_expanded: bool,
    ) -> &blocks::Block {
        let unmeasured = self.heights.get(idx).is_none_or(|h| h[0] == usize::MAX);
        if !self.slots.contains_key(&idx) || unmeasured {
            let variants =
                self.render_variants(entries, r, p, show_reasoning, tools_expanded, self.width);
            self.admit(idx, variants);
        }
        self.stamp += 1;
        let s = self.slots.get_mut(&idx).expect("just admitted");
        s.stamp = self.stamp;
        // Borrow dance: admit() may have evicted others; re-fetch is sound.
        let slot = self.slots.get(&idx).expect("hit after stamp bump");
        let group = &entries[r.start..r.end];
        let switch = block_switch(group);
        let i = Self::variant_index(slot, switch, (show_reasoning, tools_expanded));
        slot.pick(i)
    }

    /// Insert the rendered variants; evict LRU (never the newest) when
    /// the block budget would overflow.
    fn admit(&mut self, idx: usize, variants: Vec<blocks::Block>) {
        let h = variants[0].height;
        let h_on = variants.last().map(|b| b.height).unwrap_or(h);
        self.heights[idx] = [h, h_on];
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
                variants,
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
        let (b0, b1) = c.window_from_bottom(&es, &p(), true, false, 0, 40);
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
        let (b0, b1) = c.window_from_bottom(&es, &p(), true, false, 0, 40);
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
            assert_ne!(c.heights[i][0], usize::MAX, "height {i} must persist");
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

    // ---- per-variant bookkeeping (Ctrl+O / Ctrl+T) ----

    /// An assistant turn with non-empty reasoning: two variants, and the
    /// heights roster must record both — the switch flips the rendered
    /// rows and the scroll ruler together. The reasoning entry renders
    /// exactly one variant (visible form); hiding is a skip, not a
    /// re-render.
    #[test]
    fn reasoning_block_is_single_variant_and_hides_cleanly() {
        let es = vec![
            Entry::Reasoning {
                content: "推理过程，\n好几行。".into(),
            },
            Entry::Assistant {
                content: "回答正文".into(),
                usage: None,
            },
        ];
        let mut c = BlockCache::new();
        c.sync(&es, &p(), true, false, 60);
        let _ = c.rows_for(&es, &p(), true, false, 0..2);
        assert_eq!(c.slots[&0].variants.len(), 1, "reasoning 块单变体");
        assert_eq!(c.heights[0][0], c.heights[0][1], "单变体两槽高度相等");
        let shown = c.rows_for(&es, &p(), true, false, 0..2).0;
        assert!(
            shown
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("推理")))
        );
    }

    /// A tool exchange over the fold threshold: two variants, the
    /// expanded one taller.
    #[test]
    fn foldable_exchange_heights_differ_per_expand_state() {
        let es = vec![
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: "seq 1 30".into(),
                intent: String::new(),
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: (1..=30)
                    .map(|i| format!("line {i}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            },
        ];
        let mut c = BlockCache::new();
        c.sync(&es, &p(), true, false, 60);
        let _ = c.rows_for(&es, &p(), true, false, 0..1);
        let [off, on] = c.heights[0];
        assert!(on > off, "展开后块必须变高: off={off} on={on}");
        let folded = c.rows_for(&es, &p(), true, false, 0..1).0;
        let expanded = c.rows_for(&es, &p(), true, true, 0..1).0;
        assert!(expanded.len() > folded.len());
    }

    /// A tool exchange **under** the threshold renders identically in
    /// both switch positions: exactly one variant, stored once (the
    /// waste this refactor removed), and both height slots agree.
    #[test]
    fn short_exchange_stays_single_variant() {
        let es = vec![
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: "echo hi".into(),
                intent: String::new(),
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "hi".into(),
            },
        ];
        let mut c = BlockCache::new();
        c.sync(&es, &p(), true, false, 60);
        let _ = c.rows_for(&es, &p(), true, false, 0..1);
        assert_eq!(c.heights[0][0], c.heights[0][1], "单变体块两槽高度相等");
        assert_eq!(c.slots[&0].variants.len(), 1, "未超限卡片只存一份");
    }

    /// The scroll walk under a *different* switch state must use that
    /// state's heights — the old single-height bookkeeping silently
    /// measured everything folded, so the wheel drifted after Ctrl+O.
    #[test]
    fn window_walk_uses_current_variant_heights() {
        let mut es = Vec::new();
        for i in 0..10 {
            es.push(Entry::ToolRequest {
                call_id: format!("c{i}"),
                name: "bash".into(),
                args: format!("cmd{i}"),
                intent: String::new(),
            });
            es.push(Entry::ToolResult {
                call_id: format!("c{i}"),
                name: "bash".into(),
                ok: true,
                result: (1..=30)
                    .map(|j| format!("line {j}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            });
        }
        let mut c = BlockCache::new();
        c.sync(&es, &p(), true, false, 60);
        // Measure everything folded…
        let _ = c.rows_for(&es, &p(), true, false, 0..blocks::blocks(&es).len());
        // …then walk expanded: heights must come back taller than the
        // folded roster says.
        let (b0, b1) = c.window_from_bottom(&es, &p(), true, true, 0, 20);
        let acc: usize = (b0..b1)
            .map(|i| c.heights[i][1] + 1)
            .sum::<usize>()
            .saturating_sub(1);
        let folded_acc: usize = (b0..b1)
            .map(|i| c.heights[i][0] + 1)
            .sum::<usize>()
            .saturating_sub(1);
        assert!(acc >= 20, "展开态窗口行数必须覆盖视口: {acc}");
        assert!(acc > folded_acc, "展开态计高必须大于折叠态");
    }
}
