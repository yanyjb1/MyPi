// The YAML sorter: every key line and every item line is a piece, and its own indented body is what it reaches over.

use std::path::Path;

use super::LANGUAGE;
use super::scan;
use crate::server::agent::tools::fs::lang::{Line, Table};

// One file's line table.
pub(crate) fn table(_path: &Path, text: &str) -> Table {
    let raw: Vec<&str> = text.lines().collect();
    let mut out = Table::new(raw.len(), &LANGUAGE);
    // A block scalar's body is a string, not structure: every line it covers is stepped over whole.
    let mut skip = 0usize;
    for (at, line) in raw.iter().enumerate() {
        if at < skip {
            continue;
        }
        let (indent, kind) = scan::read(line);
        if kind == scan::Kind::Idle {
            continue;
        }
        let label = match kind {
            scan::Kind::Item => "item",
            _ => "key",
        };
        let exact = scan::name(line).or_else(|| Some(line.trim().to_string()));
        match scan::opens(line) {
            scan::Opens::Scalar => {
                let close = scan::scalar(&raw, at, indent);
                skip = close + 1;
                out.span(at, close, Some(label.to_string()), exact);
            }
            scan::Opens::Nested => {
                let close = scan::extent(&raw, at, indent);
                out.span(at, close, Some(label.to_string()), exact);
            }
            scan::Opens::No => out.span(at, at, Some(label.to_string()), exact),
        }
    }
    out
}

// The kind of piece this line opens: `key` or `item`.
pub(crate) fn shape(line: &Line) -> Option<&str> {
    line.label.as_deref()
}
