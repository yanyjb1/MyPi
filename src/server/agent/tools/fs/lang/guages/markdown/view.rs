// The note's narration: what it names, the notes it gathers, and who names it.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::server::agent::tools::fs::lang::Row;
use crate::server::agent::tools::fs::lang::graph::{Around, Child, Edge, Loader, Size, parts};

// Past this many notes naming this one, `up` folds them by directory.
const GROUP_AT: usize = 200;

// How many headings a child note lists before the rest fold into a count.
const HEADINGS: usize = 6;

// How far a view's own line reaches before its tail is counted.
const WIDTH: usize = 400;

pub(crate) fn view(around: &Around, _loader: &Loader, batch: &BTreeSet<PathBuf>) -> Vec<Row> {
    let base = around.base.as_path();
    let mut rows: Vec<Row> = vec![row(format!(
        "--- note {} {} ---",
        around.unit.name,
        parts::rel(base, &around.unit.place)
    ))];
    rows.push(row(format!(
        "headings ({}): {}",
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

// One note this document names: its path, its length, and the headings it opens with.
fn child_line(base: &Path, child: &Child, batch: &BTreeSet<PathBuf>) -> String {
    let mut line = format!(
        "  {} {}",
        parts::rel(base, &child.unit.place),
        match child.size {
            Size::Lines(n) => format!("{n}L"),
            Size::Files(n) => parts::plural(n, "file"),
            Size::Unknown => "?".to_string(),
        }
    );
    if batch.contains(&child.unit.place) {
        line.push_str(" [body in this batch]");
    }
    if !child.interface.is_empty() {
        let shown: Vec<String> = child.interface.iter().take(HEADINGS).cloned().collect();
        let _ = write!(line, "  {}", shown.join(" | "));
        if child.interface.len() > HEADINGS {
            let _ = write!(line, " [+{} more]", child.interface.len() - HEADINGS);
        }
    }
    clip(line)
}

// One out edge: the file it lands on, or the path the note wrote when nothing is there.
fn edge_line(base: &Path, edge: &Edge) -> String {
    let mut line = match &edge.to_file {
        Some(file) => format!(
            "  {}:{} -> {}",
            parts::rel(base, &edge.at),
            edge.line + 1,
            parts::rel(base, file)
        ),
        None => format!(
            "  {}:{} -> {}  (missing)",
            parts::rel(base, &edge.at),
            edge.line + 1,
            edge.wrote
        ),
    };
    if !edge.symbols.is_empty() {
        let _ = write!(line, "  #{}", parts::joined(&edge.symbols));
    }
    clip(line)
}

// One note naming this one.
fn up_line(base: &Path, edge: &Edge) -> String {
    let mut line = format!("{}:{}", parts::rel(base, &edge.at), edge.line + 1);
    if !edge.symbols.is_empty() {
        let _ = write!(line, " names #{}", parts::joined(&edge.symbols));
    }
    clip(line)
}

// The walk's own account: a note reaches the repository root and no further.
fn scan_line(around: &Around) -> String {
    let boundary = &around.boundary;
    let up = if boundary.no_above {
        "none (this is the repository root)"
    } else if boundary.complete {
        "repository root (complete)"
    } else {
        "budget"
    };
    format!(
        "[scan: up->{up} | down->the notes this one names | {}, {} files]",
        parts::plural(boundary.levels, "level"),
        boundary.files
    )
}

fn clip(text: String) -> String {
    let count = text.chars().count();
    if count <= WIDTH {
        return text;
    }
    let cut = text
        .char_indices()
        .nth(WIDTH)
        .map_or(text.len(), |(at, _)| at);
    format!("{}[+{}]", &text[..cut], count - WIDTH)
}
