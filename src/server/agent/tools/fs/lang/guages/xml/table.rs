// The XML sorter: an element's start and end tags are its piece, and a self-closing tag is one line of its own.

use std::path::Path;

use super::LANGUAGE;
use super::scan;
use crate::server::agent::tools::fs::lang::{Line, Table};

// One file's line table.
pub(crate) fn table(_path: &Path, text: &str) -> Table {
    let mut out = Table::new(text.lines().count(), &LANGUAGE);
    for shape in scan::shapes(text) {
        out.span(
            shape.open - 1,
            shape.close - 1,
            Some("element".to_string()),
            Some(shape.exact),
        );
    }
    out
}

// The kind of piece this line opens: `element`.
pub(crate) fn shape(line: &Line) -> Option<&str> {
    line.label.as_deref()
}
