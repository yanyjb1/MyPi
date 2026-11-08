//! Tool cards: the family modules, the shared shapes, and the one entry that lays a call.

use super::opts::Opts;
use crate::ui::tui::zone::main::history::local::content::Block;
use crate::ui::tui::zone::main::history::row::Row;

pub(crate) mod fold;
mod frame;

pub(crate) mod alias;
pub(crate) mod bash;
pub(crate) mod context;
pub(crate) mod draft;
pub(crate) mod edit;
pub(crate) mod fallback;
pub(crate) mod file;
pub(crate) mod kairos;
pub(crate) mod mcp;
pub(crate) mod memo;
pub(crate) mod partial;
pub(crate) mod task;
pub(crate) mod todo;
pub(crate) mod tree;
pub(crate) mod web;

pub(crate) use frame::*;

/// One tool call as rows at a width.
pub(crate) fn rows(block: &Block, paired: Option<&Block>, opts: &Opts, width: usize) -> Vec<Row> {
    frame::node(block, paired, opts, width)
}
