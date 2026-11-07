//! Coloring: a segment's rows at a width, spans and all.
//!
//! A segment is colored whole, so a span keeps its color across a row break.

use crate::ui::tui::zone::main::history::product::fold;
use crate::ui::tui::zone::main::history::product::segment::{Kind, Segment};
use crate::ui::tui::zone::main::history::product::table;
use crate::ui::tui::zone::main::history::row::{self, Row};

pub(crate) mod code;
pub(crate) mod prose;

/// One segment's rows at this width. `thinking` is a text block's chain styling.
pub fn rows(seg: &Segment, width: usize, thinking: bool) -> Vec<Row> {
    match seg.kind {
        Kind::Table => table::rows(seg, width),
        Kind::Code => code::rows(seg, width),
        _ => prose::rows(seg, width, thinking),
    }
}

/// Break one colored natural line at this width; continuations keep the ink.
pub fn fold_row(row: Row, width: usize) -> Vec<Row> {
    let text: String = row.runs.iter().map(|run| run.text.as_str()).collect();
    let avail = width.saturating_sub(row.lead.width()).max(1);
    row::split(&row.runs, &fold::breaks(&text, avail), row.lead)
}
