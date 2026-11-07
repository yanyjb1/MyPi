//! The run: one contiguous stretch of laid segments, whole segments in and out.

use std::collections::VecDeque;

use super::super::local::content::Block;
use super::super::local::mutable::Entry;
use super::super::local::section::{Idx, Local};
use super::super::local::slot::SlotKind;
use super::super::product::{self, Opts, Product, Products};
use super::super::row::Row;
use super::segment::{Laid, SegId};

/// What one position hands over.
enum Shape {
    Rows(Vec<Row>),
    Segs(Vec<product::Segment>, bool),
    Missing(i64),
    Dead,
}

/// What placing one segment did.
pub enum Placed {
    Laid,
    Empty,
    Missing(i64),
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

    /// Rows in the run; a gap row is one of them.
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

    /// The run row where this position's first row sits, laying from it when it is absent.
    pub fn head(
        &mut self,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        idx: Idx,
    ) -> Option<usize> {
        if !self.has(idx) {
            self.place(local, products, opts, SegId { idx, piece: 0 });
        }
        let mut at = 0usize;
        for laid in &self.segs {
            if laid.id.idx == idx && laid.id.piece == 0 {
                return Some(at);
            }
            at += laid.rows.len();
        }
        None
    }

    /// The run row this place names, when the segment is laid.
    pub fn index_of(&self, id: SegId, row: usize) -> Option<usize> {
        let mut at = 0usize;
        for laid in &self.segs {
            if laid.id == id {
                let last = laid.rows.len().saturating_sub(1);
                return Some(at + row.min(last));
            }
            at += laid.rows.len();
        }
        None
    }

    /// The place a run row names.
    pub fn at(&self, index: usize) -> Option<(SegId, usize)> {
        let mut at = 0usize;
        for laid in &self.segs {
            let end = at + laid.rows.len();
            if index < end {
                return Some((laid.id, index - at));
            }
            at = end;
        }
        None
    }

    /// The newest segment that has rows, laying nothing.
    pub fn newest(
        &mut self,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
    ) -> Option<SegId> {
        let mut idx = last_index(local)?;
        loop {
            let count = self.count(local, products, opts, idx);
            if count > 0 {
                return Some(SegId {
                    idx,
                    piece: count - 1,
                });
            }
            idx = walk::older(local, idx)?;
        }
    }

    /// The segment after this one, and how many it has with rows left.
    pub fn next(
        &mut self,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        id: SegId,
    ) -> Option<SegId> {
        let count = self.count(local, products, opts, id.idx);
        if id.piece + 1 < count {
            return Some(SegId {
                idx: id.idx,
                piece: id.piece + 1,
            });
        }
        let mut idx = walk::newer(local, id.idx)?;
        loop {
            if self.count(local, products, opts, idx) > 0 {
                return Some(SegId { idx, piece: 0 });
            }
            idx = walk::newer(local, idx)?;
        }
    }

    /// The segment before this one.
    pub fn prev(
        &mut self,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        id: SegId,
    ) -> Option<SegId> {
        if id.piece > 0 {
            return Some(SegId {
                idx: id.idx,
                piece: id.piece - 1,
            });
        }
        let mut idx = walk::older(local, id.idx)?;
        loop {
            let count = self.count(local, products, opts, idx);
            if count > 0 {
                return Some(SegId {
                    idx,
                    piece: count - 1,
                });
            }
            idx = walk::older(local, idx)?;
        }
    }

    /// One content arrival: only the tail segment of that position is laid again.
    pub fn touch(
        &mut self,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        idx: Idx,
    ) -> Vec<i64> {
        let count = self.count(local, products, opts, idx);
        let mut tail = 0usize;
        for (at, laid) in self.segs.iter().enumerate() {
            if laid.id.idx == idx {
                tail = at;
            }
        }
        let Some(laid) = self.segs.get(tail) else {
            return Vec::new();
        };
        if laid.id.idx != idx {
            return Vec::new();
        }
        let from = laid.id.piece;
        while self
            .segs
            .get(tail)
            .is_some_and(|laid| laid.id.idx == idx && laid.id.piece >= from)
        {
            self.segs.remove(tail);
        }
        let mut missing = Vec::new();
        let mut cursor = Some(SegId { idx, piece: from });
        while let Some(id) = cursor {
            if id.piece >= count {
                break;
            }
            let (rows, gone) = self.lay(local, products, opts, id);
            missing.extend(gone);
            if !rows.is_empty() {
                self.insert(Laid { id, rows });
            }
            cursor = if id.piece + 1 < count {
                Some(SegId {
                    idx,
                    piece: id.piece + 1,
                })
            } else {
                None
            };
        }
        missing
    }

    /// Lay this segment into the run.
    pub fn place(
        &mut self,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        id: SegId,
    ) -> Placed {
        if self.segs.iter().any(|laid| laid.id == id) {
            return Placed::Laid;
        }
        let (rows, missing) = self.lay(local, products, opts, id);
        if let Some(block) = missing.first() {
            return Placed::Missing(*block);
        }
        if rows.is_empty() {
            return Placed::Empty;
        }
        self.insert(Laid { id, rows });
        Placed::Laid
    }

    /// One position's rows go, wherever they sit: its content changed.
    pub fn forget(&mut self, idx: Idx) {
        self.segs.retain(|laid| laid.id.idx != idx);
    }

    /// Keep the rows between these two run rows, plus one segment of slack on each side.
    pub fn trim(&mut self, from: usize, to: usize) {
        super::trim::trim(&mut self.segs, from, to);
    }

    /// The rows of one laid segment.
    pub fn laid(&self, id: SegId) -> Option<&Laid> {
        self.segs.iter().find(|laid| laid.id == id)
    }

    fn has(&self, idx: Idx) -> bool {
        self.segs.iter().any(|laid| laid.id.idx == idx)
    }

    fn insert(&mut self, laid: Laid) {
        let at = self
            .segs
            .iter()
            .position(|other| rank(other.id) > rank(laid.id))
            .unwrap_or(self.segs.len());
        self.segs.insert(at, laid);
    }

    /// One segment's rows, the gap after it included; and the block ids it lacks.
    fn lay(
        &mut self,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        id: SegId,
    ) -> (Vec<Row>, Vec<i64>) {
        match self.shape(local, products, opts, id.idx) {
            Some(Shape::Rows(rows)) => {
                if id.piece > 0 {
                    return (Vec::new(), Vec::new());
                }
                (with_gap(rows), Vec::new())
            }
            Some(Shape::Segs(segs, thinking)) => {
                let Some(seg) = segs.get(id.piece) else {
                    return (Vec::new(), Vec::new());
                };
                let rows = super::color::rows(seg, self.width(), thinking);
                (with_gap(rows), Vec::new())
            }
            Some(Shape::Missing(block)) => (
                with_gap(vec![Row::text(
                    super::super::row::Lead::None,
                    "…",
                    Some(super::super::row::Ink::Fixed(
                        crate::ui::tui::theme::fixed::SYSTEM,
                    )),
                )]),
                vec![block],
            ),
            Some(Shape::Dead) | None => (Vec::new(), Vec::new()),
        }
    }

    fn count(
        &mut self,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        idx: Idx,
    ) -> usize {
        match self.shape(local, products, opts, idx) {
            Some(Shape::Rows(rows)) => usize::from(!rows.is_empty()),
            Some(Shape::Segs(segs, _)) => segs.len(),
            Some(Shape::Missing(_)) => 1,
            Some(Shape::Dead) | None => 0,
        }
    }

    fn shape(
        &mut self,
        local: &Local,
        products: &mut Products,
        opts: &Opts<'_>,
        idx: Idx,
    ) -> Option<Shape> {
        match idx {
            Idx::Turn(at) => match local.running().get(at as usize)? {
                Entry::Notice { text, level, .. } => {
                    Some(Shape::Rows(product::notice_rows(text, *level)))
                }
                Entry::Block(block) => {
                    let paired = local.running().block(at as usize + 1);
                    Some(turn_shape(block, paired, opts, self.width()))
                }
            },
            Idx::Fixed(num) => match &local.slot(Idx::Fixed(num))?.kind {
                SlotKind::Notice { text, level, .. } => {
                    Some(Shape::Rows(product::notice_rows(text, *level)))
                }
                SlotKind::Next => Some(Shape::Dead),
                SlotKind::Block(id) => {
                    if !local.has(*id) {
                        return Some(Shape::Missing(*id));
                    }
                    let block = local.block(Idx::Fixed(num))?;
                    let paired = local
                        .next_block(Idx::Fixed(num))
                        .filter(|next| block.answers(next));
                    let product = products.get(*id, || product::of(block, paired, opts, self.width()));
                    match product {
                        Product::Text(segs) => Some(Shape::Segs(
                            segs.clone(),
                            block.kind == super::super::local::content::Kind::Reasoning,
                        )),
                        Product::Fixed(rows) => Some(Shape::Rows(rows.clone())),
                    }
                }
            },
        }
    }
}

/// A growing position is never cached: its content changes under the cache.
fn turn_shape(
    block: &Block,
    paired: Option<&Block>,
    opts: &Opts<'_>,
    width: usize,
) -> Shape {
    match product::of(block, paired, opts, width) {
        Product::Text(segs) => Shape::Segs(
            segs,
            block.kind == super::super::local::content::Kind::Reasoning,
        ),
        Product::Fixed(rows) => Shape::Rows(rows),
    }
}

fn with_gap(mut rows: Vec<Row>) -> Vec<Row> {
    if !rows.is_empty() {
        rows.push(Row::blank());
    }
    rows
}

fn rank(id: SegId) -> (u8, i64, usize) {
    match id.idx {
        Idx::Fixed(num) => (0, num, id.piece),
        Idx::Turn(at) => (1, i64::from(at), id.piece),
    }
}

fn last_index(local: &Local) -> Option<Idx> {
    if !local.running().is_empty() {
        return Some(Idx::Turn((local.running().len() - 1) as u32));
    }
    local.newest().map(Idx::Fixed)
}

use super::walk;
