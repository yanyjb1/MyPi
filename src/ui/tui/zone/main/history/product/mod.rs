//! The product layer: one block in, pieces out, kept by block.

use std::collections::HashMap;

use super::layout::line::Row;
use super::layout::segment::Segment;

pub(crate) mod card;
pub(crate) mod markdown;
pub(crate) mod todo;

pub(crate) use card::rows as card_rows;
pub(crate) use markdown::pieces;
pub(crate) use todo::TodoPanel;

/// One piece of a text block.
pub(crate) enum Piece {
    /// Natural lines: the prefetch layer folds them.
    Natural(Vec<Row>),
    /// A table: its columns are only known at a width.
    Table(Segment),
}

/// What one block hands to the prefetch layer.
pub(crate) enum Product {
    /// Structure and natural lines; nothing here depends on a width.
    Text(Vec<Piece>),
    /// Rows laid at a width; a resize drops them.
    Fixed(Vec<Row>),
}

/// Which blocks are in hand, and how many may stay.
#[derive(Default)]
pub(crate) struct Products {
    blocks: HashMap<i64, Product>,
    order: HashMap<i64, u64>,
    seq: u64,
    margin: usize,
}

static NO_PIECES: &[Piece] = &[];
static NO_ROWS: &[Row] = &[];

impl Products {
    pub(crate) fn set_margin(&mut self, margin: usize) {
        self.margin = margin.max(1);
        self.trim();
    }

    /// A text block's pieces, laid once per block.
    pub(crate) fn text(&mut self, id: i64, make: impl FnOnce() -> Vec<Piece>) -> &[Piece] {
        if !self.blocks.contains_key(&id) {
            self.admit(id, Product::Text(make()));
        }
        match self.blocks.get(&id) {
            Some(Product::Text(pieces)) => pieces,
            _ => NO_PIECES,
        }
    }

    /// Rows already laid out at the current width.
    pub(crate) fn fixed(&mut self, id: i64, make: impl FnOnce() -> Vec<Row>) -> &[Row] {
        if !self.blocks.contains_key(&id) {
            self.admit(id, Product::Fixed(make()));
        }
        match self.blocks.get(&id) {
            Some(Product::Fixed(rows)) => rows,
            _ => NO_ROWS,
        }
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
