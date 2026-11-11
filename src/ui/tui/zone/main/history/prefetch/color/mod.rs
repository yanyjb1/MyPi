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
    rows_grouped(seg, width, thinking)
        .into_iter()
        .flatten()
        .collect()
}

/// The same rows, grouped by the source line each came from. A table is one group: its layout
/// has no source line per row.
pub fn rows_grouped(seg: &Segment, width: usize, thinking: bool) -> Vec<Vec<Row>> {
    match seg.kind {
        Kind::Table => vec![table::rows(seg, width)],
        Kind::Code => code::rows_grouped(seg, width),
        _ => prose::rows_grouped(seg, width, thinking),
    }
}

/// Break one colored natural line at this width; continuations keep the ink.
pub fn fold_row(row: Row, width: usize) -> Vec<Row> {
    let text: String = row.runs.iter().map(|run| run.text.as_str()).collect();
    let avail = width.saturating_sub(row.lead.width()).max(1);
    row::split(&row.runs, &fold::breaks(&text, avail), row.lead)
}
