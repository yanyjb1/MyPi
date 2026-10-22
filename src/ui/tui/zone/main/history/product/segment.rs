//! A run of logical lines that share one rendering kind.

use ratatui::text::Line;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Text,
    Code,
    Blank,
}

#[derive(Debug, Clone)]
pub struct Segment {
    pub kind: SegmentKind,
    pub lines: Vec<Line<'static>>,
}
