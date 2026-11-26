// The TOML sorter: each table's header and its last entry are its piece, one piece per header.

use std::path::Path;

use super::LANGUAGE;
use super::scan;
use crate::server::agent::tools::fs::lang::{Line, Table};

// One file's line table.
pub(crate) fn table(_path: &Path, text: &str) -> Table {
    let mut out = Table::new(text.lines().count(), &LANGUAGE);
    for shape in scan::shapes(text) {
        let exact = shape.name.clone();
        out.span(
            shape.open - 1,
            shape.close - 1,
            Some("table".to_string()),
            exact,
        );
    }
    out
}

// The kind of piece this line opens: `table`.
pub(crate) fn shape(line: &Line) -> Option<&str> {
    line.label.as_deref()
}
