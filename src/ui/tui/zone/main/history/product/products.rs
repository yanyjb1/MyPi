//! What one block hands to the layers below: segments, rows, or nothing.

use super::super::local::content::{Block, Kind as BlockKind};
use super::super::row::Row;
use super::opts::Opts;
use super::segment::Segment;

/// One block's product: segments, or rows laid at a width.
pub enum Product {
    /// Segments; the color layer lays each at the width it has.
    Text(Vec<Segment>),
    /// Rows laid at a width; a resize drops them.
    Fixed(Vec<Row>),
}

impl Product {
    /// Nothing to lay: a result block, or a block of empty text.
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Product::Text(segs) => segs.is_empty(),
            Product::Fixed(rows) => rows.is_empty(),
        }
    }
}

/// What one position hands to the run.
pub enum Shape<'a> {
    Segs(&'a [Segment], bool),
    Fixed(&'a [Row]),
    /// Rows built on the spot: a line this front end says.
    Rows(Vec<Row>),
    /// The position's content is not in hand.
    Missing(i64),
    /// The position draws nothing.
    Dead,
}

/// One block's product: a card, a text block, a band, a notice.
pub(crate) fn of(block: &Block, paired: Option<&Block>, opts: &Opts, width: usize) -> Product {
    match block.kind {
        BlockKind::ToolResult => Product::Fixed(Vec::new()),
        BlockKind::ToolRequest => Product::Fixed(super::card::rows(block, paired, opts, width)),
        BlockKind::User => Product::Fixed(super::user::rows(block, width)),
        BlockKind::Wake => Product::Fixed(super::wake::rows(
            block
                .text
                .as_deref()
                .or(block.content.as_deref())
                .unwrap_or(""),
        )),
        BlockKind::Assistant | BlockKind::Reasoning => super::text::product(block, opts),
    }
}
