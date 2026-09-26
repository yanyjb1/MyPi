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
//! Scrolling: `history::render_rows` walks **up from the bottom**
//! (`chat_scroll` is rows-up-from-bottom of the *canvas* — the transcript plus
//! the streaming tail, whose rows the caller reserves below the viewport
//! before cutting; this cache only ever sees transcript-space offsets).
//! Unknown heights are measured
//! on demand during that walk, so cold start renders exactly one
//! viewport. Walking into never-rendered ancient history pays a one-time
//! measure-as-you-go cost that then stays cached.
//!
//! Eviction: by **block count** (`BLOCK_BUDGET`), deliberately not by rows or
//! bytes — one block is one conversation unit (a message, a reply, a tool
//! call), and that is the granularity scroll-back wants. The trade-off is that
//! the budget bounds the *number* of cached blocks, not their size, so the
//! resident megabytes depend on what kind of blocks the transcript is made of.
//!
//! Measured with a **full bottom-to-top traversal** on a synthetic transcript
//! without the artifact ("巨物") mechanism — 200-line inline tool outputs:
//! a full cache holds ~9 000 rows at 8 000 entries and ~11 000 at 32 000, and
//! cutting the budget from 256 to 32 (≈ 1 200–1 800 rows) moves the process's
//! resident size by only **~4 MB** (8 000: 40.9 → 37.2 MB; 32 000: 72.9 →
//! 68.2 MB) — frame times do not get worse, they got marginally better. The
//! cached rows are the cheap part of that: most of what a full traversal adds
//! to RSS is allocator high-water from *rendering* the blocks at all, not the
//! rows kept afterwards. So a smaller budget does not buy megabytes, and a
//! bigger one does not cost them; the number is a *scroll-back depth* knob.
//!
//! Memory and per-frame work stay bounded by the budget, never by the
//! conversation length: scrolling into ancient history evicts ancient history.
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

use super::render::blocks::{self, Range};
use crate::server::entry::Entry;
use super::render::theme::HistoryTheme;

/// Block-count budget for cached blocks.
///
/// 256 blocks is roughly six screens per wheel page — far past any wheel
/// burst — and stays O(1) in transcript length: scrolling ancient history
/// evicts ancient history, never the transcript.
///
/// A full cache holds ~9 000–11 000 rows on un-spilled tool output. Measured
/// against a 32-block budget the whole-process difference is only **~4 MB**
/// (see the module docs) and frame times are unaffected: the budget bounds a
/// block *count*, and the resident rows are the cheap part of what rendering
/// costs. Do not "fix" a memory figure by raising this number — it is a
/// scroll-back depth knob, nothing else.
pub(crate) const BLOCK_BUDGET: usize = 256;

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

/// 反向走查的结果。
///
/// 只覆盖**真正要画的那几块**：视口下面的块不画（以前一路画到转录底部，
/// 往上翻得越远，每帧白白克隆的行越多）。调用方按 `skip_rows` 丢掉视口
/// 上方那截，再截到 viewport 行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    /// 要画的块序号区间 `[b0, b1)`。
    pub b0: usize,
    pub b1: usize,
    /// 区间开头这 `skip_rows` 行在视口**上方**，调用方丢掉。
    pub skip_rows: usize,
    /// 走查是否一路走到了第一块。走到顶 = 上面的块全都量过了。
    pub reached_top: bool,
    /// 走到顶时的**文档总行数**（块的行 + 块间空行）。没走到顶是
    /// `None`：上面还有没量过的块，总高不成立，别拿它去夹滚动位置。
    pub total_rows: Option<usize>,
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

/// 这一帧画不画这个块。
///
/// 缓存为 reasoning 只存「可见」那一份行（见 `render_variants`），所以
/// 「藏不藏」不能只在渲染分支里判——出图与量高的**每一处**都要问这里，
/// 否则 Ctrl+T 在缓存路径上是个死开关，还会留下一行空白。
fn paints(entries: &[Entry], r: Range, show_reasoning: bool) -> bool {
    match &entries[r.start] {
        Entry::Reasoning { content } => show_reasoning && !content.trim().is_empty(),
        _ => true,
    }
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
    /// 块的**身份**（`Range` = 从哪条到哪条）→ 渲染好的行。
    ///
    /// 键用 Range 而不是序号：序号会因置顶通知重排而整体漂移，也会因
    /// 「请求单独成块 → 结果到达后粘成一块」而指向另一个块，Range 不会。
    slots: HashMap<Range, Slot>,
    /// Grouping of the current transcript. Two jobs:
    ///  - **correctness**: a transcript swapped wholesale (tree navigation,
    ///    resume) with the *same* block count used to leave stale rows in
    ///    place — the length-only fingerprint could not tell the branches
    ///    apart. The generation below catches that; this list is kept so the
    ///    per-frame path can reuse it instead of re-scanning.
    ///  - **speed**: `window_from_bottom` / `rows_for` used to re-run the
    ///    O(n) grouping scan on every paint; they read this instead.
    ranges: Vec<blocks::Range>,
    /// Transcript generation (see `SessionState::transcript_generation`):
    /// bumped by the session when it **replaces** the entry list wholesale.
    /// The length check cannot see a swap to an equally-sized branch; this
    /// can. `u64::MAX` forces a rebuild on the first sync.
    generation: u64,
    /// Per-variant heights of every measured block, transcript order.
    /// Index 0 = switch OFF, 1 = switch ON (single-variant blocks keep
    /// both equal). `usize::MAX` = never rendered (never measured).
    /// Heights are derived facts, cheap to keep for all blocks (16 bytes
    /// each) and they survive eviction — scroll math over visited history
    /// stays O(1) forever.
    heights: Vec<[usize; 2]>,
    width: usize,
    /// Entry count of the synced transcript — part of the "nothing changed"
    /// fingerprint that lets an idle frame skip the O(entries) grouping scan.
    entries_len: usize,
    stamp: u64,
    /// Theme epoch the cached rows were colored with. A theme swap bumps
    /// the epoch; the next sync drops everything (heights re-derive from
    /// the re-render). omp's `themeEpoch` cache-key contract.
    theme_epoch: u64,
}

/// Which of a block's two height slots the current view state asks for.
/// Single-variant blocks keep both slots equal, so indexing by the switch is
/// safe even when the block's rows were evicted (heights outlive slots).
fn slot_of(entries: &[Entry], r: Range, show_reasoning: bool, tools_expanded: bool) -> usize {
    match block_switch(&entries[r.start..r.end]) {
        Switch::None => 0,
        Switch::Reasoning => usize::from(show_reasoning),
        Switch::Tools => usize::from(tools_expanded),
    }
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
            ranges: Vec::new(),
            generation: u64::MAX, // force a full rebuild on the first sync
            heights: Vec::new(),
            width: 0,
            entries_len: usize::MAX, // 迫使首帧真的扫一遍
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

    /// Re-sync with the transcript. Cheap when nothing changed: the block
    /// count and boundaries are the fingerprint for the append-only case,
    /// and the **generation** stamps a wholesale replacement (tree
    /// navigation / resume). Boundaries alone cannot see that swat: two
    /// branches can share every `(start, end)` index while carrying
    /// different text — exactly the navigation case.
    pub(crate) fn sync(
        &mut self,
        entries: &[Entry],
        generation: u64,
        _t: &HistoryTheme,
        _show_reasoning: bool,
        _tools_expanded: bool,
        width: usize,
    ) {
        let epoch = crate::tui::theme::theme_epoch();
        // 转录没动、宽度没变、主题没换 —— 这一帧没有一行会变。分组是
        // O(条目数) 的扫描，长对话下每敲一个键都全量扫一遍是白烧：
        // 早退，`ranges` 还是上一帧那一份。
        if generation == self.generation
            && entries.len() == self.entries_len
            && epoch == self.theme_epoch
            && width == self.width
        {
            return;
        }
        if epoch != self.theme_epoch {
            self.slots.clear();
            self.heights.clear();
            self.ranges.clear();
            self.generation = u64::MAX;
            self.theme_epoch = epoch;
        }
        if width != self.width {
            // Width change invalidates every wrap; drop rows but keep the
            // structure — heights re-derive from the re-render below.
            self.slots.clear();
            self.width = width;
            self.heights.clear();
            self.ranges.clear();
            self.generation = u64::MAX;
        }
        let ranges = blocks::blocks(entries);
        if generation != self.generation {
            // Wholesale replacement: nothing cached describes the new
            // transcript — drop rows *and* roster.
            self.slots.clear();
            self.heights.clear();
            self.heights.resize(ranges.len(), [usize::MAX; 2]);
            self.generation = generation;
        } else if ranges.len() != self.heights.len() {
            // Transcript grew (normal) or shrank (rewind).
            self.heights.resize(ranges.len(), [usize::MAX; 2]);
        }
        // 序号认不出块：置顶通知会把整条链的序号推后一位，追加一个结果会
        // 把末块从「只有调用」改写成「调用 + 结果」（块数不变）。所以逐个
        // 序号比 Range：
        //  - 变了 → 丢掉这一格的高度备忘（高度按序号存）；
        //  - 顺带把已经不在转录里的块的行清掉（行按 Range.start 认身份，
        //    序号漂移不影响命中）。
        let moved: Vec<usize> = ranges
            .iter()
            .enumerate()
            .filter(|(i, r)| self.ranges.get(*i) != Some(*r))
            .map(|(i, _)| i)
            .collect();
        if !moved.is_empty() {
            for i in &moved {
                if let Some(h) = self.heights.get_mut(*i) {
                    *h = [usize::MAX; 2];
                }
            }
            let live: std::collections::HashSet<Range> = ranges.iter().copied().collect();
            self.slots.retain(|k, _| live.contains(k));
        }
        self.ranges = ranges;
        self.entries_len = entries.len();
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
        t: &HistoryTheme,
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
                let Entry::ToolResult {
                    ok,
                    result,
                    details,
                    ..
                } = &group[1]
                else {
                    unreachable!("group of 2 is always request+result (blocks guarantees)");
                };
                let two =
                    super::render::cards::exchange_has_two_states(name, *ok, result, details.as_ref());
                ((show_reasoning, false), (show_reasoning, true), two)
            }
            _ => (
                (show_reasoning, tools_expanded),
                (show_reasoning, tools_expanded),
                false,
            ),
        };
        let mut out = Vec::with_capacity(if two { 2 } else { 1 });
        out.push(blocks::render_group(entries, r, t, off.0, off.1, width));
        if two {
            out.push(blocks::render_group(entries, r, t, on.0, on.1, width));
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

    /// The rows to paint for `range` of blocks (by ordinal). Unmeasured
    /// blocks inside the range render now; unmeasured blocks above the range
    /// do **not** render — the caller's scroll walk (reverse, from the
    /// bottom) measures those it actually crosses, and the splice in
    /// `view.rs` only ever needs the rows of this window.
    pub(crate) fn rows_for(
        &mut self,
        entries: &[Entry],
        t: &HistoryTheme,
        show_reasoning: bool,
        tools_expanded: bool,
        range: std::ops::Range<usize>,
    ) -> Vec<Line<'static>> {
        // Reuse the grouping `sync` computed this frame instead of scanning
        // the whole transcript again (`self.block` needs `&mut self`, so the
        // list is taken out and put back).
        let ranges = std::mem::take(&mut self.ranges);
        let mut out = Vec::new();
        let mut gap_needed = false;
        for (i, r) in ranges.iter().enumerate() {
            if i < range.start {
                continue;
            }
            if i >= range.end {
                break;
            }
            if !paints(entries, *r, show_reasoning) {
                continue; // 藏掉的思考：不留行，也不留间隔
            }
            let b = self.block(entries, i, *r, t, show_reasoning, tools_expanded);
            if gap_needed {
                out.push(blocks::block_gap());
            }
            out.extend(b.rows.clone());
            gap_needed = true;
        }
        self.ranges = ranges;
        out
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
        t: &HistoryTheme,
        show_reasoning: bool,
        tools_expanded: bool,
        offset_rows: usize,
        viewport_rows: usize,
    ) -> Window {
        // Reuse the grouping `sync` computed this frame (the walk calls
        // `render_variants` on `&mut self`, so the list is taken out).
        let ranges = std::mem::take(&mut self.ranges);
        let n = ranges.len();
        if n == 0 {
            self.ranges = ranges;
            return Window {
                b0: 0,
                b1: 0,
                skip_rows: 0,
                reached_top: true,
                total_rows: Some(0),
            };
        }
        // 从底部往上走，累计「当前块**之下**画了多少行」（含块间那一条
        // 间隔）。视口占据离转录底部 `[offset, offset+viewport)` 这一段：
        //  - 第一次 `below + h > offset` 的块 = 盖住视口**下沿**的块；
        //  - 第一次 `below + h >= offset + viewport` 的块 = 盖住**上沿**的块。
        // 两块之间就是要画的范围，不多画一块。
        let need = offset_rows.saturating_add(viewport_rows);
        let mut below = 0usize;
        let mut bottom: Option<usize> = None;
        let mut top: Option<usize> = None;
        for i in (0..n).rev() {
            let r = ranges[i];
            // 藏掉的块不占行、也不占间隔：跳过。它若被算进去，滚轮就会
            // 在「看不见的一行」上多停一格。
            if !paints(entries, r, show_reasoning) {
                continue;
            }
            // 走查只需要**高度**。备忘里有就别出图：这一块不在视口里，
            // 行马上用不上，窗口内那几块由 `rows_for` 按需补画。
            //
            // 以前这里对经过的每一块都调 `ensure_measured`，而走查是从
            // 转录底部一路走到视口——于是每帧代价正比于**离底部的距离**，
            // 而不是视口高度：8k 条滚到第 36000 行时每帧 ~94 ms（11 fps），
            // 越往上滚越慢。备忘之外的块仍然照旧渲染并进缓存，缓存该填满
            // 还是会填满。
            let h =
                self.height_or_measure(entries, i, r, t, show_reasoning, tools_expanded);
            // 下沿：视口最下面那一行（离转录底部 `offset` 行）落在这一块
            // 的行里，**或者**落在它上面那条间隔里——间隔属于下面那一块，
            // 所以判定要带上 `+1`，否则窗口会少画一行，调用方只能在顶部
            // 补个空白。
            if bottom.is_none() && offset_rows < below + h + 1 {
                bottom = Some(i);
            }
            if below + h >= need {
                top = Some(i);
                break;
            }
            below += h + 1; // 这一块 + 它上面那条间隔
        }
        // 没找到上沿 = 全篇都不够这个视口 → 已经走到第一块，总高成立。
        let reached_top = top.is_none();
        let b_bottom = bottom.unwrap_or(0);
        let b_top = top.unwrap_or(0);
        let (b0, b1) = (b_top.min(b_bottom), b_top.max(b_bottom) + 1);
        // 视口上方那几行 = 上沿块被视口覆盖掉的部分之后剩下的
        let skip_rows = if top.is_some() {
            below
                .saturating_add(self.height_at(b_top, ranges[b_top], entries, show_reasoning, tools_expanded))
                .saturating_sub(need)
        } else {
            0
        };
        self.ranges = ranges;
        Window {
            b0,
            b1,
            skip_rows,
            reached_top,
            // 每块都加了「+1 间隔」，块间空行只有 painted-1 条。
            total_rows: reached_top.then(|| below.saturating_sub(1)),
        }
    }

    /// 某块的当前变体高度（已经量过；调用方保证）。
    ///
    /// 注意传 `r` 而不是从 `self.ranges` 里查——走查期间 `ranges` 是被
    /// `mem::take` 出去的，`self.ranges` 此刻是空的。
    fn height_at(
        &self,
        idx: usize,
        r: Range,
        entries: &[Entry],
        show_reasoning: bool,
        tools_expanded: bool,
    ) -> usize {
        let switch = block_switch(&entries[r.start..r.end]);
        let slot_index = match switch {
            Switch::None => 0,
            Switch::Reasoning => usize::from(show_reasoning),
            Switch::Tools => usize::from(tools_expanded),
        };
        self.heights[idx][slot_index]
    }

    /// 保证这一格（`idx` 序号上的块 `r`）有行可画，并把高度备忘刷新回来。
    ///
    /// 行按 **`Range.start`** 认身份：块的序号会因置顶重排而整体漂移，
    /// 但「这一块从哪条条目开始」是稳定的。高度备忘按序号存（它要跟
    /// `heights` 一一对应），序号上的 Range 变了就已在 `sync` 里清掉。
    fn ensure_measured(
        &mut self,
        entries: &[Entry],
        idx: usize,
        r: Range,
        t: &HistoryTheme,
        show_reasoning: bool,
        tools_expanded: bool,
    ) {
        if !self.slots.contains_key(&r) {
            let variants =
                self.render_variants(entries, r, t, show_reasoning, tools_expanded, self.width);
            self.admit(r, idx, variants);
            return;
        }
        // 行还在、高度备忘没了（重排或淘汰过）→ 从行上把高度读回来，别重画。
        if self.heights.get(idx).is_none_or(|h| h[0] == usize::MAX) {
            let s = &self.slots[&r];
            let h0 = s.variants[0].height;
            let h_on = s.variants.last().map(|b| b.height).unwrap_or(h0);
            if let Some(memo) = self.heights.get_mut(idx) {
                *memo = [h0, h_on];
            }
        }
    }

    /// Height of block `idx` in `slot`, rendering it only when the memo is
    /// empty.
    ///
    /// The reverse walk needs one number per block it passes — and it passes
    /// every block between the transcript bottom and the viewport. Rendering
    /// those made a frame cost proportional to the scroll *offset* rather
    /// than to the viewport. Rows for the blocks that are actually on screen
    /// are rendered by `rows_for` through [`Self::block`].
    fn height_or_measure(
        &mut self,
        entries: &[Entry],
        idx: usize,
        r: Range,
        t: &HistoryTheme,
        show_reasoning: bool,
        tools_expanded: bool,
    ) -> usize {
        let slot = slot_of(entries, r, show_reasoning, tools_expanded);
        if let Some(h) = self.heights.get(idx).map(|h| h[slot]).filter(|h| *h != usize::MAX) {
            return h;
        }
        self.ensure_measured(entries, idx, r, t, show_reasoning, tools_expanded);
        self.heights[idx][slot]
    }

    /// One block, via the cache, in the variant the current view state
    /// asks for.
    fn block(
        &mut self,
        entries: &[Entry],
        idx: usize,
        r: Range,
        t: &HistoryTheme,
        show_reasoning: bool,
        tools_expanded: bool,
    ) -> &blocks::Block {
        self.ensure_measured(entries, idx, r, t, show_reasoning, tools_expanded);
        self.stamp += 1;
        let s = self.slots.get_mut(&r).expect("just measured");
        s.stamp = self.stamp;
        // Borrow dance: admit() may have evicted others; re-fetch is sound.
        let slot = self.slots.get(&r).expect("hit after stamp bump");
        let group = &entries[r.start..r.end];
        let switch = block_switch(group);
        let i = Self::variant_index(slot, switch, (show_reasoning, tools_expanded));
        slot.pick(i)
    }

    /// Insert the rendered variants; evict LRU (never the newest) when
    /// the block budget would overflow.
    /// 收下一块的行（键 = 块的 `Range`），并把高度写进序号备忘。
    fn admit(&mut self, r: Range, idx: usize, variants: Vec<blocks::Block>) {
        let h = variants[0].height;
        let h_on = variants.last().map(|b| b.height).unwrap_or(h);
        if let Some(memo) = self.heights.get_mut(idx) {
            *memo = [h, h_on];
        }
        // The newest block (highest start = latest entries) stays: it is
        // the follow-bottom hot path.
        let newest = self.slots.keys().map(|k| k.start).max();
        while self.slots.len() >= BLOCK_BUDGET {
            let Some(victim) = self
                .slots
                .keys()
                .copied()
                .filter(|k| Some(k.start) != newest && *k != r)
                .min_by_key(|k| self.slots[k].stamp)
            else {
                break; // budget full of pinned blocks; keep anyway
            };
            self.slots.remove(&victim);
        }
        self.stamp += 1;
        self.slots.insert(
            r,
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
    use crate::server::entry::Entry;

    fn t() -> HistoryTheme {
        HistoryTheme::resolve()
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
        c.sync(&es, 1, &t(), true, false, 60);
        let n = blocks::blocks(&es).len();
        let w = c.window_from_bottom(&es, &t(), true, false, 0, 40);
        let (b0, b1) = (w.b0, w.b1);
        let _ = c.rows_for(&es, &t(), true, false, b0..b1);
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
        c.sync(&es, 1, &t(), true, false, 60);
        // Ask for the bottom 40 rows.
        let w = c.window_from_bottom(&es, &t(), true, false, 0, 40);
        let (b0, b1) = (w.b0, w.b1);
        assert_eq!(b1, blocks::blocks(&es).len());
        let rows = c.rows_for(&es, &t(), true, false, b0..b1);
        assert!(!rows.is_empty());
    }

    #[test]
    fn cache_stays_bounded_when_walking_ancient_history() {
        let es = convo(400); // 800 entries — the "chatted for days" case
        let mut c = BlockCache::new();
        c.sync(&es, 1, &t(), true, false, 60);
        // Walk to the top through windows, like a wheel burst would.
        let n = blocks::blocks(&es).len();
        let mut top = n;
        while top > 0 {
            let b0 = top.saturating_sub(8);
            let _ = c.rows_for(&es, &t(), true, false, b0..top);
            top = b0;
        }
        assert!(
            c.cached_blocks() <= BLOCK_BUDGET,
            "LRU 必须按块数封顶：{}",
            c.cached_blocks()
        );
    }

    #[test]
    fn idle_sync_after_a_width_change_still_renders() {
        // 早退的陷阱：宽度变化会清掉行与 ranges，下一帧若因为「转录没动」
        // 就早退，ranges 会一直空着 —— 屏幕空白。这条钉住这个坑。
        let es = convo(20);
        let mut c = BlockCache::new();
        c.sync(&es, 1, &t(), true, false, 60);
        c.sync(&es, 1, &t(), true, false, 80); // 宽度变了：清行重排
        c.sync(&es, 1, &t(), true, false, 80); // 下一帧什么都没变：早退
        let w = c.window_from_bottom(&es, &t(), true, false, 0, 20);
        let (b0, b1) = (w.b0, w.b1);
        let rows = c.rows_for(&es, &t(), true, false, b0..b1);
        assert!(!rows.is_empty(), "早退之后窗口空了：ranges 没保住");
    }

    /// Ctrl+T：藏掉思考。缓存为 reasoning 只存「可见」那一份（注释说
    /// 「view 会跳过它」），所以**出图时必须真的跳过**，否则开关是死的。
    #[test]
    fn hidden_reasoning_paints_nothing() {
        let es = vec![
            Entry::User { content: "问题".into() },
            Entry::Reasoning { content: "内心独白".into() },
            Entry::Assistant { content: "答案".into(), usage: None },
        ];
        let has = |rows: &[ratatui::text::Line<'static>], needle: &str| {
            rows.iter().any(|l| l.spans.iter().any(|s| s.content.contains(needle)))
        };
        let mut c = BlockCache::new();
        // 开着：思考在
        c.sync(&es, 1, &t(), true, false, 80);
        let w = c.window_from_bottom(&es, &t(), true, false, 0, 40);
        let (b0, b1) = (w.b0, w.b1);
        assert!(has(&c.rows_for(&es, &t(), true, false, b0..b1), "内心独白"));

        // 关掉：思考不许再出现，而且不该留下空行
        c.sync(&es, 1, &t(), false, false, 80);
        let w = c.window_from_bottom(&es, &t(), false, false, 0, 40);
        let (b0, b1) = (w.b0, w.b1);
        let rows = c.rows_for(&es, &t(), false, false, b0..b1);
        assert!(!has(&rows, "内心独白"), "Ctrl+T 藏不住思考（缓存路径）");
        // 藏起来的块不该在画面上留下任何独占的行：和「转录里从来没有
        // 这条思考」渲染出来必须一模一样（块之间的空行是正当的）。
        let without: Vec<Entry> = es
            .iter()
            .filter(|e| !matches!(e, Entry::Reasoning { .. }))
            .cloned()
            .collect();
        let mut c2 = BlockCache::new();
        c2.sync(&without, 1, &t(), false, false, 80);
        let w = c2.window_from_bottom(&without, &t(), false, false, 0, 40);
        let (b0, b1) = (w.b0, w.b1);
        let plain = c2.rows_for(&without, &t(), false, false, b0..b1);
        let text = |rows: &[ratatui::text::Line<'static>]| -> String {
            rows.iter()
                .map(|l| {
                    l.spans
                        .iter()
                        .map(|s| s.content.trim_end().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(
            text(&rows),
            text(&plain),
            "藏掉思考后画面与该条不存在时不一致"
        );
    }

    /// 置顶消息到达会把它拎到最前 → 后面所有块的**序号**推后一位。
    /// 缓存若按序号认块，旧行就会挂到别的块上。
    #[test]
    fn a_pinned_notice_must_not_shift_cached_rows_onto_other_blocks() {
        let has = |rows: &[ratatui::text::Line<'static>], needle: &str| {
            rows.iter().any(|l| l.spans.iter().any(|s| s.content.contains(needle)))
        };
        let base = vec![
            Entry::User { content: "甲".into() },
            Entry::Assistant { content: "乙".into(), usage: None },
        ];
        let mut with_pin = base.clone();
        with_pin.push(Entry::System {
            text: "置顶通知".into(),
            align: crate::server::entry::Align::Center,
            pin: true,
        });
        // 前置：置顶确实排到了最前，块数 +1
        let b = blocks::blocks(&with_pin);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].start, 2, "置顶块该排在最前（指向最后一条条目）");

        let mut c = BlockCache::new();
        c.sync(&base, 1, &t(), true, false, 80);
        let r0 = c.rows_for(&base, &t(), true, false, 0..2);
        assert!(has(&r0, "甲") && has(&r0, "乙"), "前置：先缓存好内容");

        c.sync(&with_pin, 1, &t(), true, false, 80);
        let r1 = c.rows_for(&with_pin, &t(), true, false, 0..3);
        assert!(has(&r1, "置顶通知"), "置顶通知没画: {r1:?}");
        // 序号 1 现在应当是「甲」；若缓存按序号取旧行，这里会串成「乙」
        let only_jia: String = r1
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();
        assert!(only_jia.contains("甲") && only_jia.contains("乙"), "{only_jia}");
        assert_eq!(only_jia.matches("甲").count(), 1, "甲出现了不止一次: {only_jia}");
        assert_eq!(only_jia.matches("乙").count(), 1, "乙出现了不止一次: {only_jia}");
    }

    /// 画的范围必须有界：往上翻到老远的地方，窗口也只覆盖视口那几块，
    /// 不许一路画到转录底部（那会让每帧克隆几千行）。
    #[test]
    fn painted_range_stays_bounded_by_the_viewport() {
        let es = convo(400); // 800 条目，块很多
        let mut c = BlockCache::new();
        c.sync(&es, 1, &t(), true, false, 60);
        let n = blocks::blocks(&es).len();
        // 翻到很老的位置
        let w = c.window_from_bottom(&es, &t(), true, false, 900, 30);
        let rows = c.rows_for(&es, &t(), true, false, w.b0..w.b1);
        assert!(
            w.b1 - w.b0 <= 16,
            "窗口覆盖了 {} 块（{n} 块里），该只覆盖视口那几块",
            w.b1 - w.b0
        );
        assert!(
            rows.len() <= 30 + 40,
            "画了 {} 行，视口只要 30 行",
            rows.len()
        );
    }

    #[test]
    fn heights_survive_eviction() {
        // Evicted rows leave their heights behind: scroll math over visited
        // history stays exact even after the rows are gone.
        let es = convo(400);
        let mut c = BlockCache::new();
        c.sync(&es, 1, &t(), true, false, 60);
        let n = blocks::blocks(&es).len();
        let _ = c.rows_for(&es, &t(), true, false, 0..8.min(n));
        for i in 0..8.min(n) {
            assert_ne!(c.heights[i][0], usize::MAX, "height {i} must persist");
        }
    }

    #[test]
    fn content_swap_with_equal_block_count_must_not_render_stale_rows() {
        // Tree navigation / resume **replace** the whole transcript. Two
        // branches typically share the same block *shape* (same count, same
        // (start,end) indices) while carrying different text — so a
        // length- or boundary-only fingerprint cannot tell them apart, and
        // the rows of the branch you left keep painting. The generation bump
        // is what makes the swap visible.
        let mut c = BlockCache::new();
        let old: Vec<Entry> = vec![
            Entry::User {
                content: "旧用户消息".into(),
            },
            Entry::Assistant {
                content: "旧回答".into(),
                usage: None,
            },
        ];
        let new: Vec<Entry> = vec![
            Entry::User {
                content: "新用户消息".into(),
            },
            Entry::Assistant {
                content: "新回答".into(),
                usage: None,
            },
        ];
        assert_eq!(blocks::blocks(&old).len(), blocks::blocks(&new).len());
        let has = |rows: &[ratatui::text::Line<'static>], needle: &str| {
            rows.iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains(needle)))
        };
        c.sync(&old, 1, &t(), true, false, 60);
        let r0 = c.rows_for(&old, &t(), true, false, 0..2);
        assert!(has(&r0, "旧回答"), "前置：渲染旧内容");

        // Same generation would (correctly) keep the cache; the session
        // bumps it on a wholesale swap, and that is what must clear it.
        c.sync(&new, 2, &t(), true, false, 60);
        let r1 = c.rows_for(&new, &t(), true, false, 0..2);
        assert!(!has(&r1, "旧回答"), "换 transcript 后不得渲染旧分支内容");
        assert!(has(&r1, "新回答"), "必须渲染新内容");
    }

    /// 工具结果到达时，**块数不变而块内容变了**：请求单独一个块（进行中），
    /// 结果到达后两条粘成同一个块。缓存只按块数/代/宽度/主题判断失效，
    /// 四样都没动，于是旧行（还是「进行中」那张卡）继续画。
    #[test]
    fn a_result_landing_must_replace_the_pending_card() {
        let has = |rows: &[ratatui::text::Line<'static>], needle: &str| {
            rows.iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains(needle)))
        };
        let req = Entry::ToolRequest {
            call_id: "c1".into(),
            name: "bash".into(),
            args: r#"{"intent":"跑","command":"echo hi"}"#.into(),
            intent: "跑".into(),
            text: String::new(),
            first: true,
        };
        let pending = vec![req.clone()];
        let landed = vec![
            req,
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "landed".into(),
                details: None,
                duration_ms: 0,
            },
        ];
        // 前置：两种转录的块数一样
        assert_eq!(blocks::blocks(&pending).len(), blocks::blocks(&landed).len());

        let mut c = BlockCache::new();
        c.sync(&pending, 1, &t(), true, false, 60);
        let r0 = c.rows_for(&pending, &t(), true, false, 0..1);
        assert!(!has(&r0, "landed"), "前置：结果还没到");

        // 结果到了（同一代，正常 append）
        c.sync(&landed, 1, &t(), true, false, 60);
        let r1 = c.rows_for(&landed, &t(), true, false, 0..1);
        assert!(has(&r1, "landed"), "结果到了却还在画进行中那张卡: {r1:?}");
    }

    #[test]
    fn same_generation_append_keeps_measured_heights() {
        // The generation must not fire on a normal append: that path keeps
        // the heights already measured (the whole point of the roster).
        let mut c = BlockCache::new();
        let es = convo(3);
        c.sync(&es, 1, &t(), true, false, 60);
        let n = blocks::blocks(&es).len();
        let _ = c.rows_for(&es, &t(), true, false, 0..n);
        assert_ne!(c.heights[0][0], usize::MAX, "已测高度");
        // Append one more round, same generation.
        let mut grown = es.clone();
        grown.push(Entry::User {
            content: "再来".into(),
        });
        grown.push(Entry::Assistant {
            content: "好".into(),
            usage: None,
        });
        c.sync(&grown, 1, &t(), true, false, 60);
        assert_ne!(c.heights[0][0], usize::MAX, "追加不得清空已测高度");
    }

    #[test]
    fn shrink_on_rewind_truncates_roster() {
        let mut c = BlockCache::new();
        let es = convo(5);
        c.sync(&es, 1, &t(), true, false, 60);
        let _ = c.rows_for(&es, &t(), true, false, 0..10);
        let before = c.total_height();
        let cut: Vec<Entry> = es[..4].to_vec();
        c.sync(&cut, 1, &t(), true, false, 60);
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
        c.sync(&es, 1, &t(), true, false, 60);
        let _ = c.rows_for(&es, &t(), true, false, 0..2);
        // 键 = 块的 Range（身份），不是序号
        let first = blocks::blocks(&es)[0];
        assert_eq!(c.slots[&first].variants.len(), 1, "reasoning 块单变体");
        assert_eq!(c.heights[0][0], c.heights[0][1], "单变体两槽高度相等");
        let shown = c.rows_for(&es, &t(), true, false, 0..2);
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
                text: String::new(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: (1..=30)
                    .map(|i| format!("line {i}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                details: None,
                duration_ms: 0,
            },
        ];
        let mut c = BlockCache::new();
        c.sync(&es, 1, &t(), true, false, 60);
        let _ = c.rows_for(&es, &t(), true, false, 0..1);
        let [off, on] = c.heights[0];
        assert!(on > off, "展开后块必须变高: off={off} on={on}");
        let folded = c.rows_for(&es, &t(), true, false, 0..1);
        let expanded = c.rows_for(&es, &t(), true, true, 0..1);
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
                text: String::new(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "hi".into(),
                details: None,
                duration_ms: 0,
            },
        ];
        let mut c = BlockCache::new();
        c.sync(&es, 1, &t(), true, false, 60);
        let _ = c.rows_for(&es, &t(), true, false, 0..1);
        assert_eq!(c.heights[0][0], c.heights[0][1], "单变体块两槽高度相等");
        let first = blocks::blocks(&es)[0];
        assert_eq!(c.slots[&first].variants.len(), 1, "未超限卡片只存一份");
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
                text: String::new(),
                first: true,
            });
            es.push(Entry::ToolResult {
                call_id: format!("c{i}"),
                name: "bash".into(),
                ok: true,
                result: (1..=30)
                    .map(|j| format!("line {j}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                details: None,
                duration_ms: 0,
            });
        }
        let mut c = BlockCache::new();
        c.sync(&es, 1, &t(), true, false, 60);
        // Measure everything folded…
        let _ = c.rows_for(&es, &t(), true, false, 0..blocks::blocks(&es).len());
        // …then walk expanded: heights must come back taller than the
        // folded roster says.
        let w = c.window_from_bottom(&es, &t(), true, true, 0, 20);
        let (b0, b1) = (w.b0, w.b1);
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
