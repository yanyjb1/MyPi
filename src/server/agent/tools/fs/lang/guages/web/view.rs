// The web's narration of one neighbourhood: what this file offers, what hangs off it, and which files named it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::server::agent::tools::fs::lang::Row;
use crate::server::agent::tools::fs::lang::graph::{Around, Child, Edge, Loader, Size, parts};

// Past this many importers, `up` folds them by directory.
const GROUP_AT: usize = 200;

pub(crate) fn view(around: &Around, _loader: &Loader, batch: &BTreeSet<PathBuf>) -> Vec<Row> {
    let base = around.base.as_path();
    let place = format!(
        "{}{}",
        parts::rel(base, &around.unit.place),
        if around.unit.place.is_dir() { "/" } else { "" }
    );
    let mut rows: Vec<Row> = vec![row(format!("--- {} {} ---", title(around, &place), place))];
    rows.push(row(format!(
        "interface ({}): {}",
        around.interface.len(),
        parts::joined_all(&around.interface)
    )));
    if around.children.is_empty() {
        rows.push(row("down (0): none".to_string()));
    } else {
        rows.push(row(parts::heading("down", around.children.len())));
        for child in &around.children {
            rows.push(row(child_line(base, child, batch)));
        }
    }
    section(base, "out", &around.outgoing, &mut rows);
    let mut up: Vec<String> = Vec::new();
    if around.incoming.len() > GROUP_AT {
        up.extend(parts::grouped(base, &around.incoming));
    } else {
        for edge in &around.incoming {
            up.push(up_line(base, edge));
        }
    }
    if up.is_empty() {
        rows.push(row("up (0): none".to_string()));
    } else {
        rows.push(row(parts::heading("up", up.len())));
        for line in up {
            rows.push(row(format!("  {line}")));
        }
    }
    rows.push(row(scan_line(around)));
    rows
}

fn row(text: String) -> Row {
    Row {
        heading: None,
        text: parts::clip(text),
    }
}

// What this file is, in the page's own words. A unit named after its own place says it once.
fn title(around: &Around, place: &str) -> String {
    let kind = match around.unit.place.extension().and_then(|e| e.to_str()) {
        Some("html" | "htm") => "page",
        Some("css") => "stylesheet",
        Some("js") => "script",
        _ => "web",
    };
    if around.unit.name.is_empty() || around.unit.name == place {
        kind.to_string()
    } else {
        format!("{kind} {}", around.unit.name)
    }
}

fn section(base: &Path, label: &str, edges: &[Edge], rows: &mut Vec<Row>) {
    if edges.is_empty() {
        rows.push(row(format!("{label} (0): none")));
        return;
    }
    rows.push(row(parts::heading(label, edges.len())));
    for edge in edges {
        rows.push(row(edge_line(base, edge)));
    }
}

// One out edge: the file it lands on, or the path it wrote when nothing is there.
fn edge_line(base: &Path, edge: &Edge) -> String {
    let mut line = match &edge.to_file {
        Some(file) => format!(
            "  {}:{} → {}",
            parts::rel(base, &edge.at),
            edge.line + 1,
            parts::rel(base, file)
        ),
        None => format!(
            "  {}:{} → {}  (missing)",
            parts::rel(base, &edge.at),
            edge.line + 1,
            edge.wrote
        ),
    };
    if !edge.symbols.is_empty() {
        line.push_str(&format!("  {}", parts::joined(&edge.symbols)));
    }
    parts::clip(line)
}

// One child: its kind, its own surface, and whether this batch wrote it.
fn child_line(base: &Path, child: &Child, batch: &BTreeSet<PathBuf>) -> String {
    let size = match child.size {
        Size::Files(n) => parts::plural(n, "file"),
        Size::Lines(n) => format!("{n}L"),
        Size::Unknown => "?".to_string(),
    };
    let mut line = format!(
        "  {} {} {} {}",
        parts::rel(base, &child.unit.place),
        size,
        child.tags.join(" "),
        parts::plural(child.interface.len(), "name")
    );
    if batch.contains(&child.unit.place) {
        line.push_str(" [body in this batch]");
    }
    parts::clip(line)
}

// One importer, as the `up` section prints it.
fn up_line(base: &Path, edge: &Edge) -> String {
    let mut line = format!("{}:{}", parts::rel(base, &edge.at), edge.line + 1);
    if !edge.symbols.is_empty() {
        line.push_str(&format!(" names {}", parts::joined(&edge.symbols)));
    }
    parts::clip(line)
}

// The walk's own account.
fn scan_line(around: &Around) -> String {
    let boundary = &around.boundary;
    let up = if boundary.no_above {
        "none (this is the web root)"
    } else if boundary.complete {
        "web root (complete)"
    } else {
        "budget"
    };
    format!(
        "[scan: up→{up} | down→what this file names | {}, {} files]",
        parts::plural(boundary.levels, "level"),
        boundary.files
    )
}
