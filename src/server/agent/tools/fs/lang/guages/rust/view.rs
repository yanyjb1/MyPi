// Rust's narration of one neighbourhood: exports, down, out, up, and where the scan stopped.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::facts::{declares_modules, strip_attrs};
use crate::server::agent::tools::fs::lang::Row;
use crate::server::agent::tools::fs::lang::graph::{Around, Child, Edge, Loader, Size, parts};

// Past this many importers, `up` folds them by directory.
const GROUP_AT: usize = 200;

pub(crate) fn view(around: &Around, loader: &Loader, batch: &BTreeSet<PathBuf>) -> Vec<Row> {
    let base = around.base.as_path();
    let title = if around.unit.name.is_empty() {
        "crate".to_string()
    } else {
        around.unit.name.clone()
    };
    let tag = if around.unit.entry.as_deref() == Some(around.file.as_path()) {
        "entry file"
    } else {
        "primer"
    };
    let mut rows: Vec<Row> = vec![row(format!(
        "--- module {title} ({}/) · {tag} ---",
        parts::rel(base, &around.unit.place)
    ))];

    rows.push(raw_row(format!(
        "exports ({}): {}",
        around.interface.len(),
        parts::joined_all(&around.interface)
    )));

    if around.children.is_empty() {
        rows.push(row("down (0): none".to_string()));
    } else {
        rows.push(row(parts::heading("down", around.children.len())));
        for child in &around.children {
            rows.push(raw_row(child_line(around, child, batch)));
        }
    }

    let outgoing: Vec<&Edge> = around
        .outgoing
        .iter()
        .filter(|edge| !edge.re_export)
        .filter(|edge| {
            edge.to_file
                .as_ref()
                .is_none_or(|file| !parts::within(&around.unit.place, file))
        })
        .collect();
    if outgoing.is_empty() {
        rows.push(row("out (0): none".to_string()));
    } else {
        rows.push(row(parts::heading("out", outgoing.len())));
        for edge in outgoing {
            rows.push(row(out_line(base, edge)));
        }
    }

    let mut up: Vec<String> = Vec::new();
    if let Some(decl) = &around.declared_by {
        up.push(decl_line(base, decl, loader));
    }
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

    rows.push(row(scan_line(around, loader)));
    rows
}

fn raw_row(text: String) -> Row {
    Row {
        heading: None,
        text,
    }
}

fn row(text: String) -> Row {
    Row {
        heading: None,
        text: parts::clip(text),
    }
}

fn child_line(around: &Around, child: &Child, batch: &BTreeSet<PathBuf>) -> String {
    let label = match child.unit.place.strip_prefix(&around.unit.place) {
        Ok(rest) => {
            let first = rest.components().next().map_or_else(
                || child.unit.name.clone(),
                |c| c.as_os_str().to_string_lossy().into_owned(),
            );
            if child.unit.place.is_dir() {
                format!("{first}/")
            } else {
                first
            }
        }
        Err(_) => format!("{}.rs", child.unit.name),
    };
    let size = match child.size {
        Size::Files(n) => parts::plural(n, "file"),
        Size::Lines(n) => format!("{n}L"),
        Size::Unknown => "?".to_string(),
    };
    let mut line = format!(
        "  {label} {size} {} {} exports ({}): {}",
        child.tags.join(" "),
        if child.has_children { "hub" } else { "leaf" },
        child.interface.len(),
        parts::joined_all(&child.interface),
    );
    let entry = child
        .unit
        .entry
        .as_deref()
        .unwrap_or(child.unit.place.as_path());
    if batch.contains(entry) {
        line.push_str(" [body in this batch]");
    }
    let deps = deps_of(around, child);
    if !deps.is_empty() {
        let _ = write!(line, " deps: {}", deps.join(", "));
    }
    line
}

// Which siblings this child reaches into, as the line prints them.
fn deps_of(around: &Around, child: &Child) -> Vec<String> {
    // Deps are named from the child's own directory, not the one being viewed.
    let home = if child.unit.place.is_dir() {
        child.unit.place.clone()
    } else {
        child
            .unit
            .place
            .parent()
            .map_or_else(|| around.unit.place.clone(), Path::to_path_buf)
    };
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in &child.outgoing {
        let Some(to) = &edge.to_file else {
            continue;
        };
        let Ok(rest) = to.strip_prefix(&home) else {
            continue;
        };
        let Some(first) = rest.components().next() else {
            continue;
        };
        let mut name = first.as_os_str().to_string_lossy().into_owned();
        if rest.components().count() == 1 {
            name = name.strip_suffix(".rs").map(str::to_string).unwrap_or(name);
        }
        let symbols = map.entry(name).or_default();
        for symbol in &edge.symbols {
            if !symbols.contains(symbol) {
                symbols.push(symbol.clone());
            }
        }
    }
    map.into_iter()
        .map(|(name, symbols)| {
            if symbols.len() == 1 && symbols[0] == name {
                name
            } else {
                format!("{} ({})", name, symbols.join(", "))
            }
        })
        .collect()
}

fn decl_line(
    base: &Path,
    decl: &crate::server::agent::tools::fs::lang::graph::Decl,
    loader: &Loader,
) -> String {
    let from = decl.from.as_deref().unwrap_or(decl.unit.place.as_path());
    let raw = decl
        .line
        .and_then(|line| {
            loader.read(from).and_then(|text| {
                text.lines()
                    .nth(line)
                    .map(|line| strip_attrs(line.trim_start()).trim().to_string())
            })
        })
        .unwrap_or_else(|| "declares".to_string());
    format!(
        "{}:{} declares {}",
        parts::rel(base, from),
        decl.line.map_or(0, |line| line + 1),
        raw
    )
}

fn out_line(base: &Path, edge: &Edge) -> String {
    let mut line = format!(
        "  {}:{} imports {}",
        parts::rel(base, &edge.at),
        edge.line + 1,
        edge.symbols.join(", ")
    );
    if edge.to.is_none() {
        line = format!(
            "  {}:{} imports {} (external)",
            parts::rel(base, &edge.at),
            edge.line + 1,
            edge.wrote
        );
    }
    line
}

fn up_line(base: &Path, edge: &Edge) -> String {
    let mut line = format!(
        "{}:{} imports {}",
        parts::rel(base, &edge.at),
        edge.line + 1,
        edge.symbols.join(", ")
    );
    if let Some(via) = edge.via.first() {
        let _ = write!(line, " via {}", parts::rel(base, via));
    }
    line
}

fn scan_line(around: &Around, loader: &Loader) -> String {
    let boundary = &around.boundary;
    let up = if boundary.no_above {
        "none (this is the crate root)".to_string()
    } else if boundary.complete {
        "crate root (complete)".to_string()
    } else {
        match &boundary.stop {
            Some(dir) => format!("{} (budget)", parts::rel(&around.base, dir)),
            None => "stopped".to_string(),
        }
    };
    let declared = around
        .unit
        .entry
        .as_ref()
        .and_then(|entry| loader.read(entry))
        .is_some_and(|text| declares_modules(&text));
    let down = if around.children.is_empty() {
        "nothing deeper"
    } else if declared {
        "declared here"
    } else {
        "by file name"
    };
    format!(
        "[scan: up→{up} | down→{down} | {}, {} files]",
        parts::plural(boundary.levels, "level"),
        boundary.files
    )
}
