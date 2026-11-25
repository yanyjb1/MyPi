// Markdown's facts for the graph: one note is one unit, and every note or file it names hangs under it.

use std::path::{Path, PathBuf};

use super::{links, table};
use crate::server::agent::tools::fs::lang::Hub;
use crate::server::agent::tools::fs::lang::graph::{Decl, Domain, Loader, RawEdge, Unit};
use crate::server::agent::tools::workspace::canonicalize_lenient;

pub(crate) static HUB: Hub = Hub {
    edges,
    unit_of: Some(unit_of),
    territory: Some(territory),
    interface: Some(interface),
    domain: Some(domain),
    loose: Some(loose),
    view: Some(super::view::view),
};

// The suffixes of this language.
const NOTES: &[&str] = &[".md", ".markdown"];

// Whether a path is one of this language's notes.
pub(super) fn sorts(path: &Path) -> bool {
    let name = path.to_string_lossy().to_lowercase();
    NOTES.iter().any(|suffix| name.ends_with(suffix))
}

// The repository a note sits in: the nearest directory above it holding `.git`, or the note's own directory when the workspace is not a repository.
pub(super) fn repo_root(dir: &Path) -> Option<PathBuf> {
    let start = dir.canonicalize().ok()?;
    start
        .ancestors()
        .find(|at| at.join(".git").exists())
        .map(Path::to_path_buf)
        .or(Some(start))
}

// A note is a file, and one file is one unit. Its name is the name other notes call it by: the stem.
fn unit_of(path: &Path) -> Option<Unit> {
    let path = canonicalize_lenient(path);
    if !path.is_file() || !sorts(&path) {
        return None;
    }
    Some(Unit {
        name: path.file_stem()?.to_string_lossy().into_owned(),
        place: path.clone(),
        entry: Some(path),
    })
}

// The trees a note is read against: the repository it sits in, and the note's own directory as the place its relative links start from.
pub(super) fn domain(path: &Path) -> Option<Domain> {
    let path = canonicalize_lenient(path);
    let start = if path.is_dir() {
        path
    } else {
        path.parent()?.to_path_buf()
    };
    let root = repo_root(&start)?;
    Some(Domain { root, tree: start })
}

// The tree one directory owns on its own: a directory that holds a note outright and names no project of another language.
fn loose(dir: &Path) -> Option<PathBuf> {
    let holds = Loader::entries(dir)
        .into_iter()
        .any(|entry| !entry.dir && sorts(&entry.path));
    holds.then(|| dir.to_path_buf())
}

// The notes and files this note names, one declaration each, tagged by what they are.
fn territory(unit: &Unit, loader: &Loader) -> Vec<Decl> {
    let Some(text) = loader.read(&unit.place) else {
        return Vec::new();
    };
    let mut out: Vec<Decl> = Vec::new();
    for raw in edges(&unit.place, &text, loader) {
        let Some(target) = raw.targets.first() else {
            continue;
        };
        let Some(child) = unit_of(target) else {
            continue;
        };
        if child.place == unit.place || out.iter().any(|held| held.unit.place == child.place) {
            continue;
        }
        let tag = if sorts(target) { "note" } else { "file" };
        out.push(Decl {
            unit: child,
            line: Some(raw.line),
            from: Some(unit.place.clone()),
            tags: vec![tag.to_string()],
        });
    }
    out
}

// What a note offers the rest of the workspace: the headings it writes, so a parent shows what a child is about without reading it.
fn interface(_path: &Path, text: &str) -> Vec<String> {
    table::table(Path::new(""), text)
        .pieces()
        .into_iter()
        .filter(|(_, line)| table::shape(line) == Some("heading"))
        .filter_map(|(_, line)| line.exact.clone())
        .collect()
}

// Every reference this note writes, each candidate landing file its own edge: a name two notes share shows both, never the first alone.
fn edges(path: &Path, text: &str, loader: &Loader) -> Vec<RawEdge> {
    let mut out: Vec<RawEdge> = Vec::new();
    for one in links::read(path, text) {
        let targets = match links::lands(path, &one.wrote) {
            Some(found) => vec![found],
            None if one.wrote.contains('/') => Vec::new(),
            None => links::by_name(loader, path, one.wrote.trim_end_matches(".md")),
        };
        let symbols: Vec<String> = one
            .anchor
            .as_deref()
            .map(|anchor| anchored(loader, &targets, anchor))
            .into_iter()
            .collect();
        let mut first = true;
        for target in targets {
            out.push(RawEdge {
                line: one.line,
                wrote: one.wrote.clone(),
                symbols: symbols.clone(),
                targets: vec![target],
                via: Vec::new(),
                re_export: false,
            });
            first = false;
        }
        if first {
            // Nothing on disk: the edge keeps what the note wrote, so a broken link still shows.
            out.push(RawEdge {
                line: one.line,
                wrote: one.wrote.clone(),
                symbols,
                targets: Vec::new(),
                via: Vec::new(),
                re_export: false,
            });
        }
    }
    out
}

// One reference's heading, with the line that heading opens on when the landing note carries it.
fn anchored(loader: &Loader, targets: &[PathBuf], anchor: &str) -> String {
    for target in targets {
        let Some(text) = loader.read(target) else {
            continue;
        };
        for (at, line) in table::table(target, &text).pieces() {
            if table::shape(line) != Some("heading") {
                continue;
            }
            let exact = line.exact.as_deref().unwrap_or("");
            if exact.trim_start_matches('#').trim() == anchor {
                return format!("{anchor} (line {})", at + 1);
            }
        }
    }
    anchor.to_string()
}
