//! Text blocks: segments and their natural lines, and nothing about a width.

use super::super::layout::segment::{self, Kind};
use super::super::paint::{self, Opts};
use super::super::wire::decode::{Block, Kind as BlockKind};
use super::Piece;

/// One block's pieces: a fence's colors are decided whole, a table waits for the width.
pub(crate) fn pieces(block: &Block, opts: &Opts<'_>) -> Vec<Piece> {
    if block.kind == BlockKind::Reasoning && !opts.thinking {
        return vec![Piece::Natural(vec![paint::prose::placeholder(
            opts.placeholder,
        )])];
    }
    let text = block
        .content
        .as_deref()
        .or(block.text.as_deref())
        .unwrap_or_default();
    let mut out: Vec<Piece> = Vec::new();
    for seg in segment::split(text) {
        if seg.kind == Kind::Table {
            out.push(Piece::Table(seg));
            continue;
        }
        let natural = match seg.kind {
            Kind::Code => paint::code::rows(&seg),
            _ => paint::prose::text(&seg),
        };
        let natural = if block.kind == BlockKind::Reasoning {
            paint::prose::thinking(natural)
        } else {
            natural
        };
        out.push(Piece::Natural(natural));
    }
    out
}
