//! The first layer: a segment's source, one natural line per source line.
//!
//! Width plays no part here, so a resize never touches this layer.

use super::segment::Segment;

/// A segment's natural lines, in source order.
pub fn lines(seg: &Segment) -> Vec<String> {
    seg.source.lines().map(str::to_string).collect()
}
