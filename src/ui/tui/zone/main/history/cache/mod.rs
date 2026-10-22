//! The block cache: split products, laid-out copies, and heights.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use ratatui::text::Line;

use super::layout;
use super::product::Product;
use super::render::block::{self, Block};
use super::render::theme::HistoryTheme;
use crate::server::storage::BlockKind;
use crate::ui::session::BlockRef;

pub(crate) const DEFAULT_BLOCK_BUDGET: usize = 64;

#[derive(Clone, Copy)]
pub(crate) struct Item<'a> {
    pub num: i64,
    pub block: BlockRef<'a>,
    pub paired: Option<BlockRef<'a>>,
    pub version: u64,
}

struct Slot {
    products: Vec<Product>,
    variants: Vec<Block>,
    stamp: u64,
}

impl Slot {
    fn pick(&self, index: usize) -> &Block {
        &self.variants[index.min(self.variants.len() - 1)]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Switch {
    None,
    Thinking,
    Tool,
}

fn switch_of(block: BlockRef<'_>) -> Switch {
    match block.kind() {
        BlockKind::Reasoning => Switch::Thinking,
        BlockKind::ToolRequest => Switch::Tool,
        _ => Switch::None,
    }
}

fn copy_of(block: BlockRef<'_>, thinking: bool, tool: bool) -> usize {
    match switch_of(block) {
        Switch::None => 0,
        Switch::Thinking => usize::from(thinking),
        Switch::Tool => usize::from(tool),
    }
}

pub struct BlockCache {
    slots: HashMap<i64, Slot>,
    heights: HashMap<i64, [usize; 2]>,
    written: HashMap<i64, u64>,
    versions: HashMap<i64, u64>,
    tail_version: u64,
    seq: u64,
    busy: bool,
    generation: u64,
    width: usize,
    cwd: String,
    theme_epoch: u64,
    stamp: u64,
    budget: usize,
    thinking: bool,
    tool: bool,
    placeholder: String,
}

impl BlockCache {
    pub(crate) fn new() -> Self {
        Self {
            slots: HashMap::new(),
            heights: HashMap::new(),
            written: HashMap::new(),
            versions: HashMap::new(),
            tail_version: u64::MAX,
            seq: 0,
            busy: false,
            generation: u64::MAX,
            width: 0,
            cwd: String::new(),
            theme_epoch: crate::ui::tui::theme::theme_epoch(),
            stamp: 0,
            budget: DEFAULT_BLOCK_BUDGET,
            thinking: false,
            tool: false,
            placeholder: String::new(),
        }
    }

    pub(crate) fn set_budget(&mut self, n: usize) {
        self.budget = n.max(1);
    }

    pub(crate) fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    pub(crate) fn set_switches(&mut self, thinking: bool, tool: bool) {
        self.thinking = thinking;
        self.tool = tool;
    }

    pub(crate) fn set_placeholder(&mut self, text: &str) {
        self.placeholder.clear();
        self.placeholder.push_str(text);
    }

    pub(crate) fn thinking(&self) -> bool {
        self.thinking
    }

    pub(crate) fn tool(&self) -> bool {
        self.tool
    }

    pub(crate) fn clear(&mut self) {
        self.clear_all();
        self.width = 0;
    }

    fn clear_all(&mut self) {
        self.slots.clear();
        self.heights.clear();
        self.written.clear();
        self.versions.clear();
        self.tail_version = u64::MAX;
    }

    fn drop_rows(&mut self) {
        for slot in self.slots.values_mut() {
            slot.variants.clear();
        }
        self.heights.clear();
        self.written.clear();
    }

    pub(crate) fn sync(
        &mut self,
        items: &[Item<'_>],
        generation: u64,
        tail_version: u64,
        width: usize,
        cwd: &str,
    ) {
        let epoch = crate::ui::tui::theme::theme_epoch();
        if epoch != self.theme_epoch || cwd != self.cwd {
            self.clear_all();
            self.theme_epoch = epoch;
            self.cwd.clear();
            self.cwd.push_str(cwd);
            self.generation = u64::MAX;
            self.tail_version = u64::MAX;
        }
        if generation != self.generation {
            self.clear_all();
            self.generation = generation;
            self.tail_version = u64::MAX;
        }
        if width != self.width {
            self.drop_rows();
            self.width = width;
        }
        if tail_version != self.tail_version {
            for item in items {
                if item.block.is_live() {
                    self.invalidate(item.num);
                }
            }
            self.tail_version = tail_version;
        }
        let live: HashSet<i64> = items.iter().map(|item| item.num).collect();
        self.slots.retain(|k, _| live.contains(k));
        self.heights.retain(|k, _| live.contains(k));
        self.written.retain(|k, _| live.contains(k));
        self.versions.retain(|k, _| live.contains(k));
    }

    pub(crate) fn rows(&mut self, item: &Item<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
        self.ensure(item, t);
        let copy = copy_of(item.block, self.thinking, self.tool);
        self.slots
            .get(&item.num)
            .map(|slot| slot.pick(copy).rows.clone())
            .unwrap_or_default()
    }

    pub(crate) fn height(&mut self, item: &Item<'_>, t: &HistoryTheme) -> usize {
        self.ensure(item, t);
        self.heights
            .get(&item.num)
            .map(|h| h[copy_of(item.block, self.thinking, self.tool)])
            .filter(|h| *h != usize::MAX)
            .unwrap_or(0)
    }

    pub(crate) fn known_height(&self, num: i64, block: BlockRef<'_>) -> Option<usize> {
        let copy = copy_of(block, self.thinking, self.tool);
        self.heights
            .get(&num)
            .map(|h| h[copy])
            .filter(|h| *h != usize::MAX)
    }

    pub(crate) fn color_rows(&mut self, num: i64, lo: usize, hi: usize, t: &HistoryTheme) -> usize {
        let Some(slot) = self.slots.get_mut(&num) else {
            return 0;
        };
        slot.variants
            .iter_mut()
            .map(|block| block.color_rows(lo, hi, t))
            .sum()
    }

    /// Lay out this item now, if it is not already laid out.
    pub(crate) fn ensure_item(&mut self, item: &Item<'_>, t: &HistoryTheme) {
        self.ensure(item, t);
    }

    fn render_variants(&self, item: &Item<'_>, t: &HistoryTheme) -> Vec<Product> {
        let (thinking, tool) = (self.thinking, self.tool);
        let (off, on, two) = match switch_of(item.block) {
            Switch::Thinking => ((false, tool), (true, tool), true),
            Switch::Tool => {
                let two = item.paired.is_some_and(|r| {
                    super::render::cards::exchange_has_two_states(
                        item.block.name().map(String::as_str).unwrap_or_default(),
                        r.ok().unwrap_or(false),
                        r.result().map(String::as_str).unwrap_or_default(),
                        r.memo().and_then(|m| serde_json::from_str(m).ok()).as_ref(),
                    )
                });
                ((thinking, false), (thinking, true), two)
            }
            Switch::None => ((thinking, tool), (thinking, tool), false),
        };
        let mut out = Vec::with_capacity(if two { 2 } else { 1 });
        out.push(self.render_one(item, t, off));
        if two {
            out.push(self.render_one(item, t, on));
        }
        out
    }

    fn render_one(&self, item: &Item<'_>, t: &HistoryTheme, view: (bool, bool)) -> Product {
        let details = item
            .paired
            .and_then(|r| r.memo())
            .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok());
        block::render_product(
            item.block,
            item.paired,
            t,
            view.0,
            view.1,
            &self.placeholder,
            details.as_ref(),
            self.width,
            true,
            &self.cwd,
        )
    }

    fn ensure(&mut self, item: &Item<'_>, t: &HistoryTheme) {
        let mut reuse = false;
        let mut measure = false;
        let mut stale = false;
        if let Some(slot) = self.slots.get(&item.num) {
            if self.versions.get(&item.num) == Some(&item.version) {
                if slot.variants.is_empty() {
                    reuse = true;
                } else if self
                    .heights
                    .get(&item.num)
                    .is_none_or(|h| h[0] == usize::MAX)
                {
                    measure = true;
                } else {
                    return;
                }
            } else {
                stale = true;
            }
        }
        if stale {
            self.invalidate(item.num);
        }
        if reuse {
            let products = self
                .slots
                .get(&item.num)
                .map(|slot| slot.products.clone())
                .unwrap_or_default();
            let variants: Vec<Block> = products
                .iter()
                .map(|product| layout::layout(product, self.width, None))
                .collect();
            if let Some(slot) = self.slots.get_mut(&item.num) {
                slot.variants = variants;
            }
            self.measure(item.num);
            return;
        }
        if measure {
            self.measure(item.num);
            return;
        }
        let products = self.render_variants(item, t);
        self.admit(item.num, item.version, products);
    }

    fn measure(&mut self, num: i64) {
        let Some(slot) = self.slots.get(&num) else {
            return;
        };
        let h0 = slot.variants[0].height;
        let h1 = slot.variants.last().map(|block| block.height).unwrap_or(h0);
        self.heights.insert(num, [h0, h1]);
    }

    fn admit(&mut self, num: i64, version: u64, products: Vec<Product>) {
        let variants: Vec<Block> = products
            .iter()
            .map(|product| layout::layout(product, self.width, None))
            .collect();
        let h0 = variants[0].height;
        let h1 = variants.last().map(|block| block.height).unwrap_or(h0);
        self.heights.insert(num, [h0, h1]);
        self.seq += 1;
        self.written.insert(num, self.seq);
        self.versions.insert(num, version);
        if !self.busy {
            while self.slots.len() >= self.budget {
                let Some(victim) = self
                    .slots
                    .iter()
                    .filter(|(k, _)| **k != num)
                    .min_by_key(|(_, slot)| slot.stamp)
                    .map(|(k, _)| *k)
                else {
                    break;
                };
                self.slots.remove(&victim);
            }
        }
        self.stamp += 1;
        self.slots.insert(
            num,
            Slot {
                products,
                variants,
                stamp: self.stamp,
            },
        );
    }

    pub(crate) fn held_numbers(&self) -> Vec<i64> {
        self.slots
            .iter()
            .filter(|(_, slot)| !slot.variants.is_empty())
            .map(|(num, _)| *num)
            .collect()
    }

    /// Every number with a product, laid out or not.
    pub(crate) fn cached_numbers(&self) -> Vec<i64> {
        self.slots.keys().copied().collect()
    }

    pub(crate) fn has_rows(&self, num: i64) -> bool {
        self.slots
            .get(&num)
            .is_some_and(|slot| !slot.variants.is_empty())
    }

    pub(crate) fn written(&self, num: i64) -> Option<u64> {
        self.written.get(&num).copied()
    }

    /// Drop the laid-out rows but keep the split product and height.
    pub(crate) fn drop_slot(&mut self, num: i64) {
        if let Some(slot) = self.slots.get_mut(&num) {
            slot.variants.clear();
        }
        self.written.remove(&num);
    }

    /// Drop everything for one number: its product is stale.
    pub(crate) fn invalidate(&mut self, num: i64) {
        self.slots.remove(&num);
        self.heights.remove(&num);
        self.written.remove(&num);
        self.versions.remove(&num);
    }

    pub(crate) fn slice(
        &mut self,
        item: &Item<'_>,
        range: Range<usize>,
        t: &HistoryTheme,
    ) -> Vec<Line<'static>> {
        let rows = self.rows(item, t);
        rows.into_iter()
            .skip(range.start)
            .take(range.end.saturating_sub(range.start))
            .collect()
    }
}
