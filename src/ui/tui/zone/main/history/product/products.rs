//! The block cache: one entry per block, oldest admission dropped first.

use std::collections::HashMap;

use super::super::local::content::{Block, Kind as BlockKind};

use super::super::row::Row;
use super::opts::Opts;
use super::segment::Segment;

/// What one block hands to the prefetch layer.
#[derive(Clone)]
pub enum Product {
    /// Segments; the color layer lays each at the width it has.
    Text(Vec<Segment>),
    /// Rows laid at a width; a resize drops them.
    Fixed(Vec<Row>),
}

impl Product {
    /// Nothing to lay: a result block, or a block of empty text.
    fn is_empty(&self) -> bool {
        match self {
            Product::Text(segs) => segs.is_empty(),
            Product::Fixed(rows) => rows.is_empty(),
        }
    }
}

/// One cache key: the block, and the result it pairs with.
type Key = (i64, Option<i64>);

/// Which blocks are in hand, and how many may stay.
#[derive(Default)]
pub struct Products {
    blocks: HashMap<Key, Product>,
    order: HashMap<Key, u64>,
    seq: u64,
    margin: usize,
}

impl Products {
    pub(crate) fn set_margin(&mut self, margin: usize) {
        self.margin = margin.max(1);
        self.trim();
    }

    /// One block's product, built on first use. An empty one is not kept.
    pub(crate) fn get(
        &mut self,
        id: i64,
        pair: Option<i64>,
        make: impl FnOnce() -> Product,
    ) -> Product {
        let key = (id, pair);
        if let Some(held) = self.blocks.get(&key) {
            return held.clone();
        }
        let product = make();
        if !product.is_empty() {
            self.admit(key, product.clone());
        }
        product
    }

    /// Every product of this block goes, whichever result it paired with.
    pub(crate) fn drop_block(&mut self, id: i64) {
        self.blocks.retain(|key, _| key.0 != id);
        self.order.retain(|key, _| key.0 != id);
    }

    /// A resize: everything that kept a width goes.
    pub(crate) fn drop_fixed(&mut self) {
        let gone: Vec<Key> = self
            .blocks
            .iter()
            .filter(|(_, product)| matches!(*product, Product::Fixed(_)))
            .map(|(key, _)| *key)
            .collect();
        for key in gone {
            self.blocks.remove(&key);
            self.order.remove(&key);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.blocks.clear();
        self.order.clear();
        self.seq = 0;
    }

    fn admit(&mut self, key: Key, product: Product) {
        self.seq += 1;
        self.blocks.retain(|other, _| other.0 != key.0);
        self.order.retain(|other, _| other.0 != key.0);
        self.blocks.insert(key, product);
        self.order.insert(key, self.seq);
        self.trim();
    }

    fn trim(&mut self) {
        while self.blocks.len() > self.margin {
            let victim = self
                .blocks
                .keys()
                .min_by_key(|key| {
                    (
                        self.order.get(*key).copied().unwrap_or(0),
                        key.0,
                        key.1.unwrap_or(i64::MIN),
                    )
                })
                .copied();
            let Some(key) = victim else { break };
            self.blocks.remove(&key);
            self.order.remove(&key);
        }
    }
}

/// One block's product: a card, a text block, a band, a notice.
pub(crate) fn of(block: &Block, paired: Option<&Block>, opts: &Opts, width: usize) -> Product {
    match block.kind {
        BlockKind::ToolResult => Product::Fixed(Vec::new()),
        BlockKind::ToolRequest => Product::Fixed(super::card::rows(block, paired, opts, width)),
        BlockKind::User => Product::Fixed(super::user::rows(block, width)),
        BlockKind::Wake => Product::Fixed(super::wake::rows(
            block
                .text
                .as_deref()
                .or(block.content.as_deref())
                .unwrap_or(""),
        )),
        BlockKind::Assistant | BlockKind::Reasoning => super::text::product(block, opts),
    }
}
