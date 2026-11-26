// The notebook sorter: one piece per cell, and nothing else numbered.

use std::path::Path;

use super::LANGUAGE;
use super::scan;
use crate::server::agent::tools::fs::lang::{Line, Table};

// The suffixes this sorter answers to.
pub(crate) const SUFFIXES: &[&str] = &["ipynb"];

// One file's line table.
pub(crate) fn table(_path: &Path, text: &str) -> Table {
    let lines = text.lines().count();
    let mut table = Table::new(lines, &LANGUAGE);
    for cell in scan::cells(text) {
        let label = cell.kind.clone();
        let exact = format!("cell {} ({})", cell.index, cell.kind);
        table.span(cell.open - 1, cell.close - 1, Some(label), Some(exact));
    }
    table
}

// The kind of piece this line opens: `code`, `markdown` or `raw`.
pub(crate) fn shape(line: &Line) -> Option<&str> {
    line.label.as_deref()
}
