//! The split product: logical lines, their kinds, cuts, and deferred fences.

mod cut;
mod segment;

pub use cut::{CutKind, CutPoint};
pub use segment::{Segment, SegmentKind};

use super::render::block::Deferred;

#[derive(Debug, Clone)]
pub struct Product {
    pub segments: Vec<Segment>,
    pub cuts: Vec<CutPoint>,
    pub deferred: Vec<Deferred>,
}

impl Product {
    pub fn from_lines(lines: Vec<ratatui::text::Line<'static>>, deferred: Vec<Deferred>) -> Self {
        let mut segments: Vec<Segment> = Vec::new();
        let mut cuts = Vec::with_capacity(lines.len());
        for (i, line) in lines.into_iter().enumerate() {
            let code = deferred
                .iter()
                .any(|d| i >= d.before && i < d.before + d.count);
            let kind = if code {
                SegmentKind::Code
            } else if line.spans.is_empty() {
                SegmentKind::Blank
            } else {
                SegmentKind::Text
            };
            cuts.push(CutPoint {
                line: i,
                kind: CutKind::Newline,
            });
            match segments.last_mut() {
                Some(segment) if segment.kind == kind => segment.lines.push(line),
                _ => segments.push(Segment {
                    kind,
                    lines: vec![line],
                }),
            }
        }
        Self {
            segments,
            cuts,
            deferred,
        }
    }

    pub fn lines(&self) -> impl Iterator<Item = &ratatui::text::Line<'static>> {
        self.segments
            .iter()
            .flat_map(|segment| segment.lines.iter())
    }

    pub fn len(&self) -> usize {
        self.segments
            .iter()
            .map(|segment| segment.lines.len())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
