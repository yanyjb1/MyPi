//! The pin: the checklist and the ESC row, laid under the last segment.

mod esc;
mod sweep;

pub(crate) use esc::row as esc_row;

use super::super::row::Row;

/// The checklist rows first, then the ESC row when something is still running.
pub(crate) fn rows(todo: Vec<Row>, esc: Option<Row>) -> Vec<Row> {
    let mut out = todo;
    if let Some(esc) = esc {
        out.push(esc);
    }
    out
}
