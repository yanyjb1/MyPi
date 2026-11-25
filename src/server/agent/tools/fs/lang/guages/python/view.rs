// Python's narration of one neighbourhood: exports, down, out, up, and how the names are gathered.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::facts::has_pkg;
use crate::server::agent::tools::fs::lang::Row;
use crate::server::agent::tools::fs::lang::graph::{Around, Child, Edge, Loader, Size, parts};

// Past this many importers, `up` folds them by directory.
const GROUP_AT: usize = 200;

pub(crate) fn view(around: &Around, _loader: &Loader, batch: &BTreeSet<PathBuf>) -> Vec<Row> {
    let base = around.base.as_path();
    let title = if around.unit.name.is_empty() {
        "project".to_string()
    } else {
        around.unit.name.clone()
    };
    let tag = if around.unit.entry.as_deref() == Some(around.file.as_path()) {
        "entry file"
    } else {
        "module"
    };
    let place = format!(
        "{}{}",
        parts::rel(base, &around.unit.place),
        if around.unit.place.is_dir() { "/" } else { "" }
    );
    let mut rows: Vec<Row> = vec![row(format!("--- module {title} {place} · {tag} ---"))];
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
    section(base, "out", &outgoing, &mut rows);

    let mut up: Vec<String> = Vec::new();
    if let Some(decl) = &around.declared_by
        && let Some(from) = decl.from.as_deref().or(decl.unit.entry.as_deref())
    {
        up.push(format!(
            "{} declares {}",
            parts::rel(base, from),
            decl.tags.join(" ")
        ));
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
    rows.push(row(scan_line(around)));
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

fn section(base: &Path, label: &str, edges: &[&Edge], rows: &mut Vec<Row>) {
    if edges.is_empty() {
        rows.push(row(format!("{label} (0): none")));
        return;
    }
    rows.push(row(parts::heading(label, edges.len())));
    for edge in edges {
        rows.push(row(parts::edge_line(base, edge)));
    }
}

fn child_line(around: &Around, child: &Child, batch: &BTreeSet<PathBuf>) -> String {
    let label = match child.unit.place.strip_prefix(&around.unit.place) {
        Ok(rest) => rest.components().next().map_or_else(
            || child.unit.name.clone(),
            |c| c.as_os_str().to_string_lossy().into_owned(),
        ),
        Err(_) => child.unit.name.clone(),
    };
    let label = if child.unit.place.is_dir() {
        format!("{label}/")
    } else {
        label
    };
    let size = match child.size {
        Size::Files(n) => parts::plural(n, "file"),
        Size::Lines(n) => format!("{n}L"),
        Size::Unknown => "?".to_string(),
    };
    let nested = child.unit.place.is_dir() && has_pkg(&child.unit.place);
    let mut line = format!(
        "  {label} {size} {} {} exports ({}): {}",
        child.tags.join(" "),
        if nested { "hub" } else { "leaf" },
        child.interface.len(),
        parts::joined_all(&child.interface),
    );
    if batch.contains(&child.unit.place) {
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
            name = name.strip_suffix(".py").map(str::to_string).unwrap_or(name);
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

fn up_line(base: &Path, edge: &Edge) -> String {
    let mut line = format!(
        "{}:{} imports {}",
        parts::rel(base, &edge.at),
        edge.line + 1,
        parts::joined(&edge.symbols)
    );
    if let Some(via) = edge.via.first() {
        let _ = write!(line, " via {}", parts::rel(base, via));
    }
    line
}

// The walk's own account: Python reads packages by directory, so there is nothing finer than that below.
fn scan_line(around: &Around) -> String {
    let boundary = &around.boundary;
    let up = if boundary.no_above {
        "none (this is the project root)".to_string()
    } else if boundary.complete {
        "project root (complete)".to_string()
    } else {
        match &boundary.stop {
            Some(dir) => format!("{} (budget)", parts::rel(&around.base, dir)),
            None => "stopped".to_string(),
        }
    };
    format!(
        "[scan: up→{up} | down→this directory (packages by `__init__.py`) | {}, {} files]",
        parts::plural(boundary.levels, "level"),
        boundary.files
    )
}
