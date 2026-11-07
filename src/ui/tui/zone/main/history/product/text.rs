//! Text blocks: a block's segments, and the one row a folded chain shows instead.

use super::super::local::content::{Block, Kind as BlockKind};
use super::super::row::{Ink, Lead, Row};
use super::opts::Opts;
use super::products::Product;
use super::segment;
use crate::ui::tui::theme::fixed;

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
    Product::Text(segment::split(text))
}
