//! One laid segment: where it came from, and the rows it took at this width.

use super::super::local::section::Idx;
use super::super::row::Row;

/// A segment's name: the position it hangs on, and which segment of that position it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SegId {
    pub idx: Idx,
    pub piece: usize,
}

/// One segment's rows, the gap after it included.
#[derive(Debug, Clone)]
pub struct Laid {
    pub id: SegId,
    pub rows: Vec<Row>,
}
