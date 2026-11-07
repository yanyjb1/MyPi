//! The block cache: one entry per block, oldest admission dropped first.

use std::collections::HashMap;

use super::super::local::content::{Block, Kind as BlockKind};
use super::super::local::slot::Level;
use super::super::row::Row;
use super::opts::Opts;
use super::segment::Segment;

/// What one block hands to the prefetch layer.
pub enum Product {
    /// Segments; the color layer lays each at the width it has.
    Text(Vec<Segment>),
    /// Rows laid at a width; a resize drops them.
    Fixed(Vec<Row>),
}

/// Which blocks are in hand, and how many may stay.
#[derive(Default)]
pub struct Products {
    blocks: HashMap<i64, Product>,
    order: HashMap<i64, u64>,
    seq: u64,
    margin: usize,
}

impl Products {
    pub(crate) fn set_margin(&mut self, margin: usize) {
        self.margin = margin.max(1);
        self.trim();
    }

    /// One block's product, built on first use at this width.
    pub(crate) fn get(&mut self, id: i64, make: impl FnOnce() -> Product) -> &Product {
        if !self.blocks.contains_key(&id) {
            self.admit(id, make());
        }
        self.blocks.get(&id).expect("just admitted")
    }

    pub(crate) fn drop_block(&mut self, id: i64) {
        self.blocks.remove(&id);
        self.order.remove(&id);
    }

    /// A resize: everything that kept a width goes.
    pub(crate) fn drop_fixed(&mut self) {
        let gone: Vec<i64> = self
            .blocks
            .iter()
            .filter(|(_, product)| matches!(*product, Product::Fixed(_)))
            .map(|(id, _)| *id)
            .collect();
        for id in gone {
            self.drop_block(id);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.blocks.clear();
        self.order.clear();
        self.seq = 0;
    }

    fn admit(&mut self, id: i64, product: Product) {
        self.seq += 1;
        self.blocks.insert(id, product);
        self.order.insert(id, self.seq);
        self.trim();
    }

    fn trim(&mut self) {
        while self.blocks.len() > self.margin {
            let victim = self
                .order
                .iter()
                .min_by_key(|(_, stamp)| **stamp)
                .map(|(id, _)| *id);
            let Some(id) = victim else { break };
            self.drop_block(id);
        }
    }
}

/// One block's product: a card, a text block, a band, a notice.
pub(crate) fn of(block: &Block, paired: Option<&Block>, opts: &Opts, width: usize) -> Product {
    match block.kind {
        BlockKind::ToolResult => Product::Fixed(Vec::new()),
        BlockKind::ToolRequest => Product::Fixed(super::card::rows(block, paired, opts, width)),
        BlockKind::User => Product::Fixed(super::user::rows(block, width)),
        BlockKind::Wake => Product::Fixed(super::notice::rows(
            block
                .text
                .as_deref()
                .or(block.content.as_deref())
                .unwrap_or(""),
            Level::Info,
        )),
        BlockKind::Assistant | BlockKind::Reasoning => super::text::product(block, opts),
    }
}
