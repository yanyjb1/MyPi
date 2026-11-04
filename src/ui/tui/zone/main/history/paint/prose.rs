//! Markdown-class blocks: prose, the thinking chain, a card's leading text.

use ratatui::style::Modifier;

use super::super::layout::line::{Ink, Lead, Row};
use super::super::layout::segment::{self, Kind};
use super::super::layout::wrap;
use super::{code, markdown, table};

use crate::ui::tui::theme::fixed;

/// One text segment's natural rows, width-free.
pub(crate) fn text(seg: &segment::Segment) -> Vec<Row> {
    markdown::rows(&seg.source)
}

/// A whole markdown text laid out at this width, for the blocks that are drawn whole.
pub(crate) fn inline(text: &str, width: usize) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    for seg in segment::split(text) {
        let rows = match seg.kind {
            Kind::Code => code::rows(&seg),
            Kind::Table => table::rows(&seg, width),
            _ => markdown::rows(&seg.source),
        };
        out.extend(wrap::rows(rows, width));
    }
    out
}

/// The thinking chain's own look: muted and leaning.
pub(crate) fn thinking(rows: Vec<Row>) -> Vec<Row> {
    rows.into_iter()
        .map(|mut row| {
            for run in &mut row.runs {
                run.ink = Some(Ink::Fixed(fixed::THINKING));
                run.attrs |= Modifier::ITALIC;
            }
            row
        })
        .collect()
}

/// What a folded thinking chain shows instead of itself.
pub(crate) fn placeholder(text: &str) -> Row {
    Row::text(Lead::First, text, Some(Ink::Fixed(fixed::SYSTEM)))
}
