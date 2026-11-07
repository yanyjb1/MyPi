//! The prefetch layer: the laid segments around the sight line.

use std::collections::VecDeque;

use super::super::local::content::Block;
use super::super::local::section::{Idx, Local};
use super::super::local::slot::SlotKind;
use super::super::product::{self, Opts, Products};
use super::super::row::Row;
use super::segment::{At, Laid};
use super::walk;

/// What one position hands over: whole rows, or segments to color.
enum Shape {
    Rows(Vec<Row>),
    Segs(Vec<product::Segment>, bool),
    Dead,
}

/// The run of laid segments, and the width it was laid at.
#[derive(Default)]
pub struct Run {
    segs: VecDeque<Laid>,
    width: usize,
}

impl Run {
    /// The width moved: every laid row goes, and every width-keeping product with it.
    pub fn set_width(&mut self, width: usize, products: &mut Products) {
        if self.width == width {
            return;
        }
        self.segs.clear();
        self.width = width;
        products.drop_fixed();
    }

    pub fn width(&self) -> usize {
        self.width.max(1)
    }

    pub fn clear(&mut self) {
        self.segs.clear();
    }

    /// Rows in the run; the gap after each segment is one of them.
    pub fn len(&self) -> usize {
        self.segs.iter().map(|laid| laid.rows.len()).sum()
    }

    /// One row of the run.
    pub fn row(&self, index: usize) -> Option<&Row> {
        let mut at = index;
        for laid in &self.segs {
            if at < laid.rows.len() {
                return laid.rows.get(at);
            }
            at -= laid.rows.len();
        }
        None
    }

    /// The run index of a slot's first content row.
    pub fn head(&self, idx: Idx) -> Option<usize> {
        let mut at = 0usize;
        for laid in &self.segs {
            if laid.idx == idx {
                return Some(at);
            }
            at += laid.rows.len();
        }
        None
    }

    /// The run index a place names.
    pub fn index_of(&self, at: At) -> Option<usize> {
        let mut base = 0usize;
        for laid in &self.segs {
            if laid.idx == at.idx && laid.piece == at.piece {
                let last = laid.rows.len().saturating_sub(1);
                return Some(base + at.row.min(last));
            }
            base += laid.rows.len();
        }
        None
    }

    /// The place a run index names.
    pub fn at_of(&self, index: usize) -> Option<At> {
        let mut base = 0usize;
        for laid in &self.segs {
            let end = base + laid.rows.len();
            if index < end {
                return Some(At {
                    idx: laid.idx,
                    piece: laid.piece,
                    row: index - base,
                });
            }
            base = end;
        }
        None
    }

    /// One more segment on the newer end: the growing one is re-laid, new ones follow.
    pub fn extend_newer(&mut self, local: &Local, products: &mut Products, opts: &Opts) -> bool {
        let Some(last) = self.segs.back() else {
            return self.start(local, products, opts);
        };
        let (idx, piece) = (last.idx, last.piece);
        if last.open {
            let (rows, open) = self
                .lay(local, products, opts, idx, piece)
                .unwrap_or_default();
            if self.segs.back().is_some_and(|laid| laid.rows != rows) {
                let Some(back) = self.segs.back_mut() else {
                    return false;
                };
                back.rows = rows;
                back.open = open;
                return true;
            }
            return false;
        }
        let mut cursor = (idx, piece);
        loop {
            let Some((next, count)) = self.step_newer(local, products, opts, cursor.0, cursor.1)
            else {
                return false;
            };
            if next.1 >= count {
                return false;
            }
            cursor = next;
            if let Some((rows, open)) = self.lay(local, products, opts, cursor.0, cursor.1) {
                self.push_back(Laid {
                    idx: cursor.0,
                    piece: cursor.1,
                    rows,
                    open,
                });
                return true;
            }
        }
    }

    /// One more segment on the older end.
    pub fn extend_older(&mut self, local: &Local, products: &mut Products, opts: &Opts) -> bool {
        let Some(first) = self.segs.front() else {
            return self.start(local, products, opts);
        };
        let (idx, piece) = (first.idx, first.piece);
        let mut cursor = (idx, piece);
        loop {
            let Some((prev, _)) = self.step_older(local, products, opts, cursor.0, cursor.1) else {
                return false;
            };
            cursor = prev;
            if let Some((rows, open)) = self.lay(local, products, opts, cursor.0, cursor.1) {
                self.push_front(Laid {
                    idx: cursor.0,
                    piece: cursor.1,
                    rows,
                    open,
                });
                return true;
            }
        }
    }

    /// No segment is laid: begin at the newest one there is.
    fn start(&mut self, local: &Local, products: &mut Products, opts: &Opts) -> bool {
        let Some(idx) = local.newest().map(Idx::Fixed) else {
            return self.start_live(local, products, opts);
        };
        self.lay_from(local, products, opts, idx);
        if self.segs.is_empty() {
            return self.start_live(local, products, opts);
        }
        true
    }

    /// A conversation with nothing finished yet: start at the running turn.
    fn start_live(&mut self, local: &Local, products: &mut Products, opts: &Opts) -> bool {
        if local.live().is_empty() {
            return false;
        }
        self.lay_from(local, products, opts, Idx::Live(0));
        !self.segs.is_empty()
    }

    /// Lay from this slot's first segment on, forwards.
    pub fn lay_from(&mut self, local: &Local, products: &mut Products, opts: &Opts, idx: Idx) {
        self.segs.clear();
        let mut cursor = (idx, 0usize);
        loop {
            if let Some((rows, open)) = self.lay(local, products, opts, cursor.0, cursor.1) {
                self.push_back(Laid {
                    idx: cursor.0,
                    piece: cursor.1,
                    rows,
                    open,
                });
                return;
            }
            let Some((next, count)) = self.step_newer(local, products, opts, cursor.0, cursor.1)
            else {
                return;
            };
            if next.1 >= count {
                return;
            }
            cursor = next;
        }
    }

    /// Keep the segments the window needs, whole.
    pub fn trim(&mut self, from: usize, to: usize) {
        super::trim::trim(&mut self.segs, from, to);
    }

    /// Drop a slot's rows, wherever they sit.
    pub fn forget(&mut self, idx: Idx) {
        self.segs.retain(|laid| laid.idx != idx);
    }

    fn push_back(&mut self, mut laid: Laid) {
        if !laid.rows.is_empty() {
            laid.rows.push(Row::blank());
        }
        self.segs.push_back(laid);
    }

    fn push_front(&mut self, mut laid: Laid) {
        if !laid.rows.is_empty() {
            laid.rows.push(Row::blank());
        }
        self.segs.push_front(laid);
    }

    /// One segment's rows, and whether it can still grow.
    fn lay(
        &mut self,
        local: &Local,
        products: &mut Products,
        opts: &Opts,
        idx: Idx,
        piece: usize,
    ) -> Option<(Vec<Row>, bool)> {
        let shape = shape(local, products, opts, idx, self.width())?;
        match shape {
            Shape::Rows(rows) => (piece == 0).then_some((rows, matches!(idx, Idx::Live(_)))),
            Shape::Segs(segs, thinking) => {
                let seg = segs.get(piece)?;
                let rows = super::color::rows(seg, self.width(), thinking);
                let open = piece + 1 == segs.len() && matches!(idx, Idx::Live(_));
                Some((rows, open))
            }
            Shape::Dead => None,
        }
    }

    /// The next (segment, count) inside the slot, or the next non-empty slot.
    fn step_newer(
        &mut self,
        local: &Local,
        products: &mut Products,
        opts: &Opts,
        idx: Idx,
        piece: usize,
    ) -> Option<((Idx, usize), usize)> {
        let count = self.count(local, products, opts, idx);
        if piece + 1 < count {
            return Some(((idx, piece + 1), count));
        }
        let mut cursor = walk::newer(local, idx);
        while let Some(next) = cursor {
            let count = self.count(local, products, opts, next);
            if count > 0 {
                return Some(((next, 0), count));
            }
            cursor = walk::newer(local, next);
        }
        None
    }

    fn step_older(
        &mut self,
        local: &Local,
        products: &mut Products,
        opts: &Opts,
        idx: Idx,
        piece: usize,
    ) -> Option<((Idx, usize), usize)> {
        if piece > 0 {
            return Some(((idx, piece - 1), 0));
        }
        let mut cursor = walk::older(local, idx);
        while let Some(prev) = cursor {
            let count = self.count(local, products, opts, prev);
            if count > 0 {
                return Some(((prev, count - 1), count));
            }
            cursor = walk::older(local, prev);
        }
        None
    }

    fn count(&mut self, local: &Local, products: &mut Products, opts: &Opts, idx: Idx) -> usize {
        match shape(local, products, opts, idx, self.width()) {
            Some(Shape::Rows(_)) => 1,
            Some(Shape::Segs(segs, _)) => segs.len(),
            _ => 0,
        }
    }
}

/// What one position hands over, at this width.
fn shape(
    local: &Local,
    products: &mut Products,
    opts: &Opts,
    idx: Idx,
    width: usize,
) -> Option<Shape> {
    match idx {
        Idx::Live(at) => {
            let block = local.live().get(at as usize)?;
            let paired = local.live().get(at as usize + 1);
            Some(live_shape(block, paired, opts, width))
        }
        Idx::Fixed(num) => match &local.slot(Idx::Fixed(num))?.kind {
            SlotKind::Notice { text, level, .. } => {
                Some(Shape::Rows(product::notice_rows(text, *level)))
            }
            SlotKind::Next => Some(Shape::Dead),
            SlotKind::Block(id) => {
                let block = local.block(Idx::Fixed(num))?;
                let paired = local
                    .next_block(Idx::Fixed(num))
                    .filter(|next| block.answers(next));
                let product = products.get(*id, || product::of(block, paired, opts, width));
                match product {
                    product::Product::Text(segs) => Some(Shape::Segs(
                        segs.clone(),
                        block.kind == super::super::local::content::Kind::Reasoning,
                    )),
                    product::Product::Fixed(rows) => Some(Shape::Rows(rows.clone())),
                }
            }
        },
    }
}

/// A growing block is never cached: its content changes under the cache.
fn live_shape(block: &Block, paired: Option<&Block>, opts: &Opts, width: usize) -> Shape {
    match product::of(block, paired, opts, width) {
        product::Product::Text(segs) => Shape::Segs(
            segs,
            block.kind == super::super::local::content::Kind::Reasoning,
        ),
        product::Product::Fixed(rows) => Shape::Rows(rows),
    }
}
