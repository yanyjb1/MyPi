//! One laid segment, and a place inside the run.

use super::super::local::section::Idx;
use super::super::row::Row;

/// One segment's rows at the width the run keeps, plus the gap after it.
#[derive(Debug, Clone)]
pub struct Laid {
    pub idx: Idx,
    pub piece: usize,
    pub rows: Vec<Row>,
    /// The block is still growing: this segment is re-laid on every arrival.
    pub open: bool,
}

/// A place inside the run: the slot, the segment, and the row inside the segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct At {
    pub idx: Idx,
    pub piece: usize,
    pub row: usize,
}
