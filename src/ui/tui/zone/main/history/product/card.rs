//! Whole-block products: cards and the front end's own messages, laid at a width.

use super::super::layout::line::Row;
use super::super::paint::{self, Opts};
use super::super::wire::decode::{Block, Kind};

/// Rows for a block that has no natural lines to fold.
pub(crate) fn rows(
    block: &Block,
    paired: Option<&Block>,
    opts: &Opts<'_>,
    width: usize,
) -> Vec<Row> {
    match block.kind {
        Kind::ToolRequest => paint::card::node(block, paired, opts, width),
        Kind::User | Kind::Wake => paint::user::block(block, width),
        _ => Vec::new(),
    }
}
