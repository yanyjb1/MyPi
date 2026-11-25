// The upward walk: one whole level at a time under a deadline, or the whole tree once. A level that runs out of time is dropped whole: half a level reads as "nobody else points here".

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::index::unit_texts;
use super::{Boundary, Domain, Loader, RawEdge, Unit};
use crate::server::agent::tools::fs::lang::Hub;

// How long a walk may spend past its first level. The first level always finishes.
const BUDGET: Duration = Duration::from_millis(300);

// Walk the domain and collect every file's raw edges, with the boundary that was reached.
pub(crate) fn scan(
    facts: &Hub,
    loader: &Loader,
    unit: &Unit,
    domain: &Domain,
) -> (Vec<(PathBuf, Vec<RawEdge>)>, Boundary) {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for (path, _) in unit_texts(unit, loader) {
        seen.insert(path);
    }
    levels(facts, loader, unit, domain, &mut seen)
}

// A file the facts' language answers to. Other files are not this walk's business.
fn claimed(facts: &Hub, path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str());
    match crate::server::agent::tools::fs::lang::suffixes(facts) {
        Some(suffixes) => ext.is_some_and(|ext| suffixes.contains(&ext)),
        None => true,
    }
}

fn root_of(unit: &Unit) -> PathBuf {
    if unit.place.is_dir() {
        unit.place.clone()
    } else {
        unit.place
            .parent()
            .map_or_else(|| unit.place.clone(), Path::to_path_buf)
    }
}

fn levels(
    facts: &Hub,
    loader: &Loader,
    unit: &Unit,
    domain: &Domain,
    seen: &mut HashSet<PathBuf>,
) -> (Vec<(PathBuf, Vec<RawEdge>)>, Boundary) {
    let dir = root_of(unit);
    let deadline = Instant::now() + BUDGET;
    let mut found: Vec<(PathBuf, Vec<RawEdge>)> = Vec::new();
    let mut levels = 0usize;
    let mut files = 0usize;
    let mut stop = None;
    let mut complete = false;
    let mut level = 1;
    while let Some(base) = dir.ancestors().nth(level) {
        if !base.starts_with(&domain.root) {
            break;
        }
        let mut roots = vec![base.to_path_buf()];
        if let Some(up) = base.parent()
            && up.starts_with(&domain.root)
        {
            for entry in Loader::entries(up) {
                if entry.dir && entry.path != base {
                    roots.push(entry.path);
                }
            }
        }
        let first = level == 1;
        let mut level_hits: Vec<(PathBuf, Vec<RawEdge>)> = Vec::new();
        let mut level_files = 0usize;
        let mut over = false;
        for root in &roots {
            if !walk(
                facts,
                loader,
                unit,
                root,
                seen,
                &mut level_hits,
                &mut level_files,
                first,
                deadline,
            ) {
                over = true;
                break;
            }
        }
        if over {
            break;
        }
        found.extend(level_hits);
        files += level_files;
        levels += 1;
        // One level covers its base and the base's peers, so the walk has reached the parent.
        let reached = match base.parent() {
            Some(up) if up.starts_with(&domain.root) => up,
            _ => base,
        };
        if reached == domain.root {
            complete = true;
            stop = None;
            break;
        }
        stop = Some(reached.to_path_buf());
        if base == domain.root {
            break;
        }
        level += 1;
        if Instant::now() >= deadline {
            break;
        }
    }
    (
        found,
        Boundary {
            levels,
            files,
            stop,
            complete,
            no_above: false,
        },
    )
}

// One level's tree. Past the first level, a tree that runs out of time is dropped whole.
#[allow(clippy::too_many_arguments)]
fn walk(
    facts: &Hub,
    loader: &Loader,
    unit: &Unit,
    root: &Path,
    seen: &mut HashSet<PathBuf>,
    hits: &mut Vec<(PathBuf, Vec<RawEdge>)>,
    files: &mut usize,
    first: bool,
    deadline: Instant,
) -> bool {
    let skip = root_of(unit);
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if !first && Instant::now() >= deadline {
            return false;
        }
        // A unit that is one file names only that file: its neighbours are exactly who may point at it, so they are walked.
        if unit.place.is_dir() && dir == skip {
            continue;
        }
        for entry in Loader::entries(&dir) {
            if entry.dir {
                // The same unit rule one level down: a directory unit never walks into itself.
                if entry.path != skip || !unit.place.is_dir() {
                    stack.push(entry.path);
                }
                continue;
            }
            if !seen.insert(entry.path.clone()) {
                continue;
            }
            if !claimed(facts, &entry.path) {
                continue;
            }
            *files += 1;
            if let Some(text) = loader.read(&entry.path) {
                let edges = (facts.edges)(&entry.path, &text, loader);
                if !edges.is_empty() {
                    hits.push((entry.path, edges));
                }
            }
        }
    }
    true
}
