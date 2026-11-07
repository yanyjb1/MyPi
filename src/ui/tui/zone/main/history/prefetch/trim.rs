//! Whole segments leave; a segment is never half kept.

use std::collections::VecDeque;

use super::segment::Laid;

/// Keep the segments the screen touches, whole; a segment entirely outside it goes.
pub fn trim(segs: &mut VecDeque<Laid>, from: usize, to: usize) {
    if segs.is_empty() {
        return;
    }
    let mut base = 0usize;
    let mut drop_front = 0usize;
    let mut keep = segs.len();
    for (at, laid) in segs.iter().enumerate() {
        let end = base + laid.rows.len();
        if end <= from {
            drop_front = at + 1;
        }
        if base >= to {
            keep = at.max(drop_front);
            break;
        }
        base = end;
    }
    for _ in 0..drop_front {
        segs.pop_front();
    }
    let keep = keep.saturating_sub(drop_front);
    while segs.len() > keep {
        segs.pop_back();
    }
}
