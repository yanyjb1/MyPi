//! The run: one contiguous stretch of laid segments, whole segments in and out.

use std::collections::VecDeque;

use super::super::local::section::{Idx, Local};
use super::super::product::{Opts, Shape};
use super::super::row::Row;
use super::segment::{Laid, SegId};

/// What laying one position's first row found.
pub enum Head {
    /// This segment holds the position's first row.
    Laid(SegId),
    /// The position's content is not in hand.
    Missing(i64),
    /// The position draws nothing.
    Empty,
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
    /// The rows pinned under the newest segment: rebuilt every draw, never cached.
    pin: Vec<Row>,
    width: usize,
}

impl Run {
    /// The width moved: every laid row goes.
    pub fn set_width(&mut self, width: usize) {
        if self.width == width {
            return;
        }
        self.segs.clear();
        self.width = width;
    }

    pub fn width(&self) -> usize {
        self.width.max(1)
    }

    pub fn clear(&mut self) {
        self.segs.clear();
    }

    /// Rows in the run; a gap row is one of them.
    pub fn len(&self) -> usize {
        self.body_len() + self.pin.len()
    }

    /// Rows the segments take, the pin excluded.
    pub fn body_len(&self) -> usize {
        self.segs.iter().map(|laid| laid.rows.len()).sum()
    }

    /// Rows the pin takes.
    pub fn pin_len(&self) -> usize {
        self.pin.len()
    }

    /// One pinned row.
    pub fn pin_row(&self, at: usize) -> Option<&Row> {
        self.pin.get(at)
    }

    /// Replace the pinned rows under the newest segment.
    pub fn set_pin(&mut self, rows: Vec<Row>) {
        self.pin = rows;
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
        self.pin.get(at)
    }

    /// Whether this position still has rows in the run.
    pub fn holds(&self, idx: Idx) -> bool {
        self.segs.iter().any(|laid| laid.id.idx == idx)
    }

    /// The position's first row, laying whole pieces until one has rows.
    pub fn head(&mut self, local: &mut Local, opts: &Opts<'_>, idx: Idx) -> Head {
        let count = self.count(local, opts, idx);
        for piece in 0..count {
            let id = SegId { idx, piece };
            match self.place(local, opts, id) {
                Placed::Laid => return Head::Laid(id),
                Placed::Missing(block) => return Head::Missing(block),
                Placed::Empty => {}
            }
        }
        Head::Empty
    }

    /// The place `rows` above this one, laying rows the run does not hold. Stops at the
    /// oldest content instead of padding above it.
    pub fn back(
        &mut self,
        local: &mut Local,
        opts: &Opts<'_>,
        mut id: SegId,
        mut row: usize,
        mut rows: usize,
    ) -> ((SegId, usize), Vec<i64>) {
        let mut missing = Vec::new();
        while rows > 0 {
            if row > 0 {
                let take = row.min(rows);
                row -= take;
                rows -= take;
                continue;
            }
            let Some(prev) = self.prev(local, opts, id) else {
                break;
            };
            match self.place(local, opts, prev) {
                Placed::Missing(block) => {
                    missing.push(block);
                    return ((prev, 0), missing);
                }
                Placed::Empty => {
                    id = prev;
                    continue;
                }
                Placed::Laid => {}
            }
            let len = self.laid(prev).map_or(0, |laid| laid.rows.len());
            if len > rows {
                return ((prev, len - rows), missing);
            }
            rows -= len;
            id = prev;
        }
        ((id, row), missing)
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
    pub fn newest(&mut self, local: &mut Local, opts: &Opts<'_>) -> Option<SegId> {
        let mut idx = last_index(local)?;
        loop {
            let count = self.count(local, opts, idx);
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
    pub fn next(&mut self, local: &mut Local, opts: &Opts<'_>, id: SegId) -> Option<SegId> {
        let count = self.count(local, opts, id.idx);
        if id.piece + 1 < count {
            return Some(SegId {
                idx: id.idx,
                piece: id.piece + 1,
            });
        }
        let mut idx = walk::newer(local, id.idx)?;
        loop {
            if self.count(local, opts, idx) > 0 {
                return Some(SegId { idx, piece: 0 });
            }
            idx = walk::newer(local, idx)?;
        }
    }

    /// The segment before this one.
    pub fn prev(&mut self, local: &mut Local, opts: &Opts<'_>, id: SegId) -> Option<SegId> {
        if id.piece > 0 {
            return Some(SegId {
                idx: id.idx,
                piece: id.piece - 1,
            });
        }
        let mut idx = walk::older(local, id.idx)?;
        loop {
            let count = self.count(local, opts, idx);
            if count > 0 {
                return Some(SegId {
                    idx,
                    piece: count - 1,
                });
            }
            idx = walk::older(local, idx)?;
        }
    }

    /// Lay this segment into the run.
    pub fn place(&mut self, local: &mut Local, opts: &Opts<'_>, id: SegId) -> Placed {
        if self.segs.iter().any(|laid| laid.id == id) {
            return Placed::Laid;
        }
        let (rows, missing) = self.lay(local, opts, id);
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

    fn insert(&mut self, laid: Laid) {
        let at = self
            .segs
            .iter()
            .position(|other| rank(other.id) > rank(laid.id))
            .unwrap_or(self.segs.len());
        self.segs.insert(at, laid);
    }

    /// One segment's rows, the gap after it included; and the block ids it lacks.
    fn lay(&mut self, local: &mut Local, opts: &Opts<'_>, id: SegId) -> (Vec<Row>, Vec<i64>) {
        let width = self.width();
        local.prepare(id.idx, opts, width);
        match local.shape(id.idx) {
            Shape::Fixed(rows) => {
                if id.piece > 0 {
                    return (Vec::new(), Vec::new());
                }
                (with_gap(rows.to_vec()), Vec::new())
            }
            Shape::Rows(rows) => (with_gap(rows), Vec::new()),
            Shape::Segs(segs, thinking) => {
                let Some(seg) = segs.get(id.piece) else {
                    return (Vec::new(), Vec::new());
                };
                let rows = super::color::rows(seg, width, thinking);
                (with_gap(rows), Vec::new())
            }
            Shape::Missing(block) => (
                with_gap(vec![Row::text(
                    super::super::row::Lead::None,
                    "…",
                    Some(super::super::row::Ink::Fixed(
                        crate::ui::tui::theme::fixed::SYSTEM,
                    )),
                )]),
                vec![block],
            ),
            Shape::Dead => (Vec::new(), Vec::new()),
        }
    }

    /// The segments this position takes; `1` while its content is missing.
    fn count(&mut self, local: &mut Local, opts: &Opts<'_>, idx: Idx) -> usize {
        let width = self.width();
        local.count(idx, opts, width)
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
