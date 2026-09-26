//! Bounded LRU cache of rendered transcript **blocks**.
//!
//! The cache unit is the block (one user message / assistant turn / tool
//! exchange / system notice), not the row. Each slot stores the rendered
//! rows for every **variant** the block actually has: a block reacts to at
//! most one view switch (tool exchanges to Ctrl+O's `tools_expanded`, the
//! reasoning block to Ctrl+T), and only when its content actually differs
//! between the switch's two positions (a short tool output renders
//! identically folded and expanded — one variant, one render, one copy).
//! Row height is a *derived* property of the rendered rows — nothing is
//! measured ahead of time, nothing is rendered "just to measure and thrown
//! away". Blocks the viewport never touches are never rendered at all.
//!
//! **A block is addressed by its key, not by its position.** For a stored
//! block that key is its `block_id` from the server; for the still-live tail
//! (entries that are not rows yet) it is a negative id the history zone
//! hands out. Positions are useless here: the front end's window *slides*,
//! so the same block sits at a different index on every page fetched.
//! Keys also make eviction trivial — nothing has to be renumbered.
//!
//! Memory and per-frame work stay bounded by the **budget**, never by the
//! conversation length: the window slides, and blocks that leave it leave
//! the cache with it. The budget is a *scroll-back depth* knob
//! ([`BlockCache::set_budget`], wired to `tui.renderMargin`), not a memory
//! lever: measured on a full bottom-to-top traversal, cutting it from 256
//! to 32 moved the process's resident size by only ~4 MB and did not slow
//! frames down. The resident rows are the cheap part of what rendering
//! costs — and most of what a full traversal adds to RSS is allocator
//! high-water from rendering the blocks at all.
//!
//! Invalidation is wholesale on width / theme-epoch / generation change
//! (every wrap is wrong, or the transcript was replaced). Beyond that the
//! cache only ever *prunes*: keys that left the window drop their rows.

use std::collections::{HashMap, HashSet};

use ratatui::text::Line;

use super::render::blocks::{self, Range};
use super::render::theme::HistoryTheme;
use crate::server::entry::Entry;

/// 渲染行缓存的默认预算（块数）。**可配**：`tui.renderMargin` 就是它 ——
/// 视口周围留多少块的行是画好待用的。
pub(crate) const DEFAULT_BLOCK_BUDGET: usize = 64;

/// 缓存面对的一块：**键**（身份）+ 条目。
///
/// 键对已落盘的块就是 `block_id`（服务端给的），对还没落盘的尾巴是历史区
/// 自己发的负数。位置不能用：窗口一滑，同一块的下标就换了。
#[derive(Clone, Copy)]
pub(crate) struct Item<'a> {
    pub key: i64,
    pub entries: &'a [Entry],
}

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

/// 这一帧画不画这个块。
///
/// 缓存为 reasoning 只存「可见」那一份行（见 `render_variants`），所以
/// 「藏不藏」不能只在渲染分支里判——出图与量高的**每一处**都要问这里，
/// 否则 Ctrl+T 在缓存路径上是个死开关，还会留下一行空白。
pub(crate) fn paints(entries: &[Entry], show_reasoning: bool) -> bool {
    match entries.first() {
        Some(Entry::Reasoning { content }) => show_reasoning && !content.trim().is_empty(),
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

/// Which of a block's two height slots the current view state asks for.
/// Single-variant blocks keep both slots equal, so indexing by the switch is
/// safe even when the block's rows were evicted (heights outlive slots).
fn slot_of(entries: &[Entry], show_reasoning: bool, tools_expanded: bool) -> usize {
    match block_switch(entries) {
        Switch::None => 0,
        Switch::Reasoning => usize::from(show_reasoning),
        Switch::Tools => usize::from(tools_expanded),
    }
}

pub struct BlockCache {
    /// (从缓存取行, 现渲染) 的累计次数——诊断"滚动到底在不在重渲染"。
    pub diag: (u64, u64),
    /// 块键 → 渲染好的行。
    slots: HashMap<i64, Slot>,
    /// 块键 → 两个视图状态下的高度（`usize::MAX` = 没量过）。行被淘汰之后
    /// 高度还留着：走查只要高度，命中率比行低不掉线。
    heights: HashMap<i64, [usize; 2]>,
    /// Transcript generation (see `SessionState::transcript_generation`):
    /// bumped when the session **replaces** the transcript wholesale (branch
    /// switch, resume, compaction). `u64::MAX` forces a rebuild on the first
    /// sync.
    generation: u64,
    width: usize,
    /// Theme epoch the cached rows were colored with. A theme swap bumps the
    /// epoch; the next sync drops everything. omp's `themeEpoch` contract.
    theme_epoch: u64,
    stamp: u64,
    /// Rendered-block budget（`tui.renderMargin`）。
    budget: usize,
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

    /// 渲染行的块数预算（`tui.renderMargin`）。
    pub(crate) fn budget(&self) -> usize {
        self.budget
    }

    /// 换预算（配置进来）。收窄时不立刻淘汰，下一次 `sync` 顺手收拾。
    pub(crate) fn set_budget(&mut self, n: usize) {
        self.budget = n.max(8);
    }

    pub(crate) fn new() -> Self {
        Self {
            slots: HashMap::new(),
            heights: HashMap::new(),
            generation: u64::MAX, // force a full rebuild on the first sync
            width: 0,
            theme_epoch: crate::tui::theme::theme_epoch(),
            stamp: 0,
            budget: DEFAULT_BLOCK_BUDGET,
            diag: (0, 0),
        }
    }

    /// 跟窗口对齐：换代/宽度/主题就整片作废，否则只把**离开窗口的键**丢掉。
    pub(crate) fn sync(&mut self, items: &[Item<'_>], generation: u64, width: usize) {
        let epoch = crate::tui::theme::theme_epoch();
        if epoch != self.theme_epoch || width != self.width {
            // 每一行的折行/配色都作废了。
            self.slots.clear();
            self.heights.clear();
            self.theme_epoch = epoch;
            self.width = width;
            self.generation = u64::MAX; // 下一句按整体替换走
        }
        if generation != self.generation {
            self.slots.clear();
            self.heights.clear();
            self.generation = generation;
        }
        // 只留窗口里还有的键：窗口滑动 = 这里滑走。
        let live: HashSet<i64> = items.iter().map(|i| i.key).collect();
        self.slots.retain(|k, _| live.contains(k));
        self.heights.retain(|k, _| live.contains(k));
    }

    /// Render the block's **actual** variants (1 or 2). Returns the
    /// rendered forms in order: `[switch-off]` or `[switch-off, switch-on]`
    /// — empty never happens (a painted block always renders).
    ///
    /// A block reacts to at most one switch: a tool exchange's two states
    /// exist only when the rendered rows actually differ (that is what
    /// `exchange_has_two_states` is the truth for). A reasoning block's two
    /// states are *visible vs hidden* — hidden renders zero rows and we do
    /// not cache emptiness — so it is single-variant too. Everything else
    /// is single-variant, rendered exactly once.
    fn render_variants(
        &self,
        entries: &[Entry],
        t: &HistoryTheme,
        show_reasoning: bool,
        tools_expanded: bool,
        width: usize,
    ) -> Vec<blocks::Block> {
        let r = Range {
            start: 0,
            end: entries.len(),
        };
        let (off, on, two) = match entries.first() {
            // Reasoning blocks are single-variant: Ctrl+T hides the block
            // outright (the history zone skips it), so the cache only ever
            // stores the visible form.
            Some(Entry::Reasoning { .. }) => ((true, tools_expanded), (true, tools_expanded), false),
            Some(Entry::ToolRequest { name, .. }) if entries.len() == 2 => {
                let Entry::ToolResult {
                    ok,
                    result,
                    details,
                    ..
                } = &entries[1]
                else {
                    unreachable!("group of 2 is always request+result (grouping guarantees)");
                };
                let two = super::render::cards::exchange_has_two_states(
                    name,
                    *ok,
                    result,
                    details.as_ref(),
                );
                ((show_reasoning, false), (show_reasoning, true), two)
            }
            _ => (
                (show_reasoning, tools_expanded),
                (show_reasoning, tools_expanded),
                false,
            ),
        };
        let mut out = Vec::with_capacity(if two { 2 } else { 1 });
        // 缓存那一路**推迟上色**：首帧只出纯文本，段落进视口时再补
        // （见 `blocks::Deferred` 与 `HistoryZone::color_visible`）。
        out.push(blocks::render_group(entries, r, t, off.0, off.1, width, true));
        if two {
            out.push(blocks::render_group(entries, r, t, on.0, on.1, width, true));
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

    /// 给这一块落在块内行区间 `[lo, hi)` 的待上色段补色，返回补了几段。
    ///
    /// **变体全补**：视图开关（Ctrl+O / Ctrl+T）随时可能翻，翻过去那一边也
    /// 得有颜色。两边的段各自按自己的行区间对（它们的排版不同，行区间本来就
    /// 不一样）。
    pub(crate) fn color_rows(
        &mut self,
        key: i64,
        lo: usize,
        hi: usize,
        t: &HistoryTheme,
    ) -> usize {
        let Some(slot) = self.slots.get_mut(&key) else {
            return 0;
        };
        slot.variants
            .iter_mut()
            .map(|b| b.color_rows(lo, hi, t))
            .sum()
    }

    /// 还有几段待上色（诊断用）。
    pub(crate) fn pending_rows(&self) -> usize {
        self.slots.values().map(|s| {
            s.variants.iter().map(|b| b.pending_len()).sum::<usize>()
        }).sum()
    }

    /// 这一块在当前视图状态下的行数（画不出来的块 = 0）。
    ///
    /// 走查只要高度：备忘里有就别出图。
    pub(crate) fn height(
        &mut self,
        it: &Item<'_>,
        t: &HistoryTheme,
        show_reasoning: bool,
        tools_expanded: bool,
    ) -> usize {
        if !paints(it.entries, show_reasoning) {
            return 0;
        }
        let slot = slot_of(it.entries, show_reasoning, tools_expanded);
        if let Some(h) = self.heights.get(&it.key).map(|h| h[slot]).filter(|h| *h != usize::MAX) {
            return h;
        }
        self.ensure_measured(it, t, show_reasoning, tools_expanded);
        self.heights[&it.key][slot]
    }

    /// 备忘里已经量过的高度；没量过就 `None`（**不出图**）。
    ///
    /// 给走查的"多看几块"用：视口那几块必须量（不然算不出视口在哪），
    /// 视口之外的多看只花备忘里有的——不然一次滚动撞上几个巨型卡片，
    /// 一帧就得替它们付整张卡的排版钱（实测峰值 1 s）。
    pub(crate) fn known_height(
        &self,
        it: &Item<'_>,
        show_reasoning: bool,
        tools_expanded: bool,
    ) -> Option<usize> {
        if !paints(it.entries, show_reasoning) {
            return Some(0);
        }
        let slot = slot_of(it.entries, show_reasoning, tools_expanded);
        self.heights
            .get(&it.key)
            .map(|h| h[slot])
            .filter(|h| *h != usize::MAX)
    }

    /// 一批块的行，按 `order`（窗口下标）给出的顺序拼起来，块间补空行。
    ///
    /// 上层只把**真正要画的那几块**放进来：视口外的块不画，往上翻得越远，
    /// 每帧白白克隆的行就越少。
    pub(crate) fn rows_for(
        &mut self,
        items: &[Item<'_>],
        order: &[usize],
        t: &HistoryTheme,
        show_reasoning: bool,
        tools_expanded: bool,
    ) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        let mut gap_needed = false;
        for &i in order {
            let Some(it) = items.get(i) else { continue };
            if !paints(it.entries, show_reasoning) {
                continue; // 藏掉的思考：不留行，也不留间隔
            }
            if self.slots.contains_key(&it.key) {
                self.diag.0 += 1;
            }
            self.ensure_measured(it, t, show_reasoning, tools_expanded);
            if gap_needed {
                out.push(blocks::block_gap());
            }
            let switch = block_switch(it.entries);
            self.stamp += 1;
            if let Some(s) = self.slots.get_mut(&it.key) {
                s.stamp = self.stamp;
            }
            let i = self
                .slots
                .get(&it.key)
                .map(|s| Self::variant_index(s, switch, (show_reasoning, tools_expanded)))
                .unwrap_or(0);
            if let Some(b) = self.slots.get(&it.key).map(|s| s.pick(i)) {
                out.extend(b.rows.clone());
            }
            gap_needed = true;
        }
        out
    }

    /// 保证这一块有行可画，并把高度备忘刷新回来。
    fn ensure_measured(
        &mut self,
        it: &Item<'_>,
        t: &HistoryTheme,
        show_reasoning: bool,
        tools_expanded: bool,
    ) {
        if !self.slots.contains_key(&it.key) {
            let variants =
                self.render_variants(it.entries, t, show_reasoning, tools_expanded, self.width);
            self.admit(it.key, variants);
            return;
        }
        // 行还在、高度备忘没了 → 从行上把高度读回来，别重画。
        if self
            .heights
            .get(&it.key)
            .is_none_or(|h| h[0] == usize::MAX)
        {
            let s = &self.slots[&it.key];
            let h0 = s.variants[0].height;
            let h_on = s.variants.last().map(|b| b.height).unwrap_or(h0);
            self.heights.insert(it.key, [h0, h_on]);
        }
    }

    /// Insert the rendered variants; evict LRU when the budget would overflow.
    fn admit(&mut self, key: i64, variants: Vec<blocks::Block>) {
        // 真正出图一次就记一次——走查里为量高而渲染的那些也算（它们是这一帧
        // 真实付掉的排版工作）。
        self.diag.1 += 1;
        let h = variants[0].height;
        let h_on = variants.last().map(|b| b.height).unwrap_or(h);
        self.heights.insert(key, [h, h_on]);
        while self.slots.len() >= self.budget {
            let Some(victim) = self
                .slots
                .iter()
                .filter(|(k, _)| **k != key)
                .min_by_key(|(_, s)| s.stamp)
                .map(|(k, _)| *k)
            else {
                break; // the budget cannot hold even one block; keep what is here
            };
            self.slots.remove(&victim);
        }
        self.stamp += 1;
        self.slots.insert(
            key,
            Slot {
                variants,
                stamp: self.stamp,
            },
        );
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
