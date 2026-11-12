//! Text blocks: a block's segments, and the one row a folded chain shows instead.

use super::super::local::content::{Block, Kind as BlockKind};
use super::super::row::{Ink, Lead, Row};
use super::opts::Opts;
use super::products::Product;
use super::segment::{self, Segment};
use crate::ui::tui::theme::fixed;

/// A text block's segments: the closed ones, and the open tail an append resumes at.
///
/// A new piece only re-splits the tail; the closed segments never move again.
#[derive(Default)]
pub struct Text {
    segs: Vec<Segment>,
    /// The byte the tail starts at: the point the next append re-splits from.
    tail_from: usize,
}

impl Text {
    pub fn new(text: &str) -> Text {
        let cuts = segment::split_from(text, 0);
        Text {
            tail_from: cuts.last().map_or(0, |cut| cut.at),
            segs: cuts.into_iter().map(|cut| cut.seg).collect(),
        }
    }

    /// One more piece landed: the tail is split again, its closed head joins the rest.
    pub fn grow(&mut self, text: &str) {
        if text.len() <= self.tail_from {
            return;
        }
        let cuts = segment::split_from(&text[self.tail_from..], self.tail_from);
        if cuts.is_empty() {
            return;
        }
        self.tail_from = cuts.last().map_or(self.tail_from, |cut| cut.at);
        self.segs.truncate(self.tail_index());
        self.segs.extend(cuts.into_iter().map(|cut| cut.seg));
    }

    pub fn segs(&self) -> &[Segment] {
        &self.segs
    }

    /// The open tail's position: the run lays this piece and everything after it again.
    pub fn tail_index(&self) -> usize {
        self.segs.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }
}

/// A text block's product: its segments, or the placeholder of a folded chain.
pub fn product(block: &Block, opts: &Opts) -> Product {
    if block.kind == BlockKind::Reasoning && !opts.thinking {
        return Product::Fixed(vec![Row::text(
            Lead::First,
            opts.placeholder,
            Some(Ink::Fixed(fixed::SYSTEM)),
        )]);
    }
    let text = block
        .content
        .as_deref()
        .or(block.text.as_deref())
        .unwrap_or_default();
    Product::Text(Text::new(text))
}
