// The language registry: the contract every tool asks, and the one list of languages.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::server::platform::stop::Stop;

use super::graph::{self, Around, Decl, Domain, Loader, RawEdge, Unit};
use super::guages;
use super::table::{Line, Table};
use super::verdict::{Check, Outcome, Verdict};

// How one neighbourhood reads on screen.
type View = fn(&Around, &Loader, &BTreeSet<PathBuf>) -> Vec<Row>;

// What a language knows about its units and their edges. Only `edges` is required: with no edges there is no graph, and the other slots degrade one at a time.
pub(crate) struct Hub {
    // What one file reaches, already resolved to landing files.
    pub(crate) edges: fn(&Path, &str, &Loader) -> Vec<RawEdge>,

    // The unit a file belongs to.
    pub(crate) unit_of: Option<fn(&Path) -> Option<Unit>>,

    // The units one unit holds, and how the parent writes each.
    pub(crate) territory: Option<fn(&Unit, &Loader) -> Vec<Decl>>,

    // What a file exposes, one printable signature per line.
    pub(crate) interface: Option<fn(&Path, &str) -> Vec<String>>,

    // Where to look for whoever points at a unit.
    pub(crate) domain: Option<fn(&Path) -> Option<Domain>>,

    // The tree one directory owns on its own, when it holds no marker and is still a project of this language.
    pub(crate) loose: Option<fn(&Path) -> Option<PathBuf>>,

    // How to print one neighbourhood; the machine has a default.
    pub(crate) view: Option<View>,
}

// One text's turn through the parser a language names: whether it parses, and what the parser said when it does not. The text is never rewritten.
pub(crate) type Format = fn(&str, &Path, &Arc<Stop>) -> Outcome;

// What every language owes the tools: the files that mark its projects, the suffixes it answers to, its table, the answers about a piece, and the external tools it knows how to drive.
pub(crate) struct Language {
    // The files that mark a project of this language in one directory; a sweep of the workspace stops at one.
    pub(crate) markers: &'static [&'static str],
    pub(crate) suffixes: &'static [&'static str],
    // The sorter, handed the path with the text: a language covering several file kinds reads the kind from the name.
    pub(crate) table: fn(&Path, &str) -> Table,
    pub(crate) shape: fn(&Line) -> Option<&str>,

    // The relations it can index: `A::b` members and a name's own declarations; `None` when it names nothing.
    pub(crate) relations: Option<&'static Relations>,

    // What it knows about units, edges and neighbourhoods; `None` when it has no module view.
    pub(crate) hub: Option<&'static Hub>,

    // Whether its files are source: they take the source budget and are read whole.
    pub(crate) source: bool,
    pub(crate) format: Option<Format>,
    pub(crate) check: Option<Checker>,
    pub(crate) test: Option<Tests>,
}

// How one language indexes relations: given the whole crate's sources, it reads a query and answers the rows that declare what it names. A language's own affair, never the tool's.
pub(crate) struct Relations {
    // The rows a query answers, and the source indices they drew from.
    pub(crate) scan: fn(&[Source], &Path, &str) -> Relation,
}

// One source a relation reads: its path, text and line table.
pub(crate) struct Source<'a> {
    pub(crate) path: &'a Path,
    pub(crate) text: &'a str,
    pub(crate) table: &'a Table,
}

// What a relation answered: the rows it read, and the source indices they drew from.
pub(crate) struct Relation {
    pub(crate) rows: Vec<Row>,
    pub(crate) used: Vec<usize>,
}

// One line of a relation's answer. A heading stands alone; every other row is a text line.
pub(crate) struct Row {
    pub(crate) heading: Option<String>,
    pub(crate) text: String,
}

// How one language's checker finds a build unit and asks it for its diagnostics.
pub(crate) struct Checker {
    // The unit a file belongs to; `None` when no unit holds it.
    pub(crate) unit: fn(&Path) -> Option<PathBuf>,

    // What a report calls that unit.
    pub(crate) title: fn(&Path) -> String,

    // How that unit names one of its files, so a diagnostic can be matched to a write.
    pub(crate) shown: fn(&Path, &Path) -> String,

    // Check one unit, given the files this batch wrote into it.
    pub(crate) run: fn(&Path, &[PathBuf], &Arc<Stop>) -> Check,
}

// The tests one text declares inside these 1-based line ranges, by name.
type Find = fn(&str, &Table, &[(usize, usize)]) -> Vec<String>;

// How one language finds the tests a batch just wrote and runs those alone.
pub(crate) struct Tests {
    // The unit a file belongs to; `None` when no unit holds it.
    pub(crate) unit: fn(&Path) -> Option<PathBuf>,
    // What a report calls that unit.
    pub(crate) title: fn(&Path) -> String,
    // The tests this text declares inside these 1-based line ranges, by name.
    pub(crate) find: Find,
    // Run exactly these tests in that unit, given the files this batch wrote into it.
    pub(crate) run: fn(&Path, &[PathBuf], &[String], &Arc<Stop>) -> Verdict,
}

// Every sorted language, in the order a suffix meets them.
static LANGUAGES: &[&Language] = &[
    &guages::rust::LANGUAGE,
    &guages::python::LANGUAGE,
    &guages::web::LANGUAGE,
    &guages::markdown::LANGUAGE,
];

// The language that claims this path, by its suffix.
pub(crate) fn owner(path: &Path) -> Option<&'static Language> {
    let suffix = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    LANGUAGES
        .iter()
        .copied()
        .find(|one| one.suffixes.contains(&suffix))
}

// The graph knowledge this path's language supplies, when it has any.
pub(crate) fn hub(path: &Path) -> Option<&'static Hub> {
    owner(path)?.hub
}

// The suffixes of the language a hub belongs to, for a walk that must not read foreign files.
pub(crate) fn suffixes(hub: &Hub) -> Option<&'static [&'static str]> {
    LANGUAGES
        .iter()
        .copied()
        .find(|one| one.hub.is_some_and(|owned| std::ptr::eq(owned, hub)))
        .map(|one| one.suffixes)
}

// The source trees the workspace holds, one entry per project of a language this build knows. A tree at or above the workspace root stands for the whole workspace, so a project whose root is the workspace root is dropped: what the workspace root already covers needs no claim. When nothing is left, the workspace root is the one tree.
pub(crate) fn trees(workspace: &Path) -> Vec<PathBuf> {
    let root = crate::server::agent::tools::workspace::canonicalize_lenient(workspace);
    let mut found: Vec<PathBuf> = Vec::new();
    let settled = vec![false; LANGUAGES.len()];
    sweep_projects(&root, &mut found, &settled);
    found.retain(|tree| tree.starts_with(&root) && tree != &root);
    found.sort();
    found.dedup();
    // The outermost of two nested trees covers the inner one whole, so the inner one is dropped.
    let mut out: Vec<PathBuf> = Vec::new();
    for tree in found {
        if out.iter().any(|held| tree.starts_with(held)) {
            continue;
        }
        out.retain(|held| !held.starts_with(&tree));
        out.push(tree);
    }
    if out.is_empty() {
        out.push(root);
    }
    out
}

// One directory's turn in the sweep. A language's marker claims the project at this directory and stops that language's search below it; a language still unclaimed by any marker takes a directory that holds its files outright as one of its own trees. `settled` carries, as one flag per language, the languages whose project already stands above this directory.
fn sweep_projects(dir: &Path, found: &mut Vec<PathBuf>, settled: &[bool]) {
    if dir.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name == "target" || name == "node_modules" || name.starts_with('.')
    }) {
        return;
    }
    let mut settled = settled.to_vec();
    let mut claimed = false;
    for (at, one) in LANGUAGES.iter().enumerate() {
        if settled[at] || !one.markers.iter().any(|mark| dir.join(mark).is_file()) {
            continue;
        }
        settled[at] = true;
        claimed = true;
        let Some(domain) = one.hub.and_then(|hub| hub.domain) else {
            continue;
        };
        if let Some(held) = domain(dir) {
            found.push(held.tree);
        }
    }
    if !claimed {
        for (at, one) in LANGUAGES.iter().enumerate() {
            if settled[at] {
                continue;
            }
            let Some(claim) = one.hub.and_then(|hub| hub.loose) else {
                continue;
            };
            if let Some(tree) = claim(dir) {
                found.push(tree);
                settled[at] = true;
            }
        }
    }
    for entry in graph::loader::entries(dir) {
        if entry.dir {
            sweep_projects(&entry.path, found, &settled);
        }
    }
}

// Every file under one directory that some sorter claims, recursively; junk skipped.
pub(crate) fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    sweep(dir, &mut out);
    out.sort();
    out
}

fn sweep(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in graph::loader::entries(dir) {
        if entry.dir {
            sweep(&entry.path, out);
        } else if owner(&entry.path).is_some() {
            out.push(entry.path);
        }
    }
}

// One file's line table: the sorter its suffix names, or every line as its own piece.
pub(crate) fn table(path: &Path, text: &str) -> Table {
    match owner(path) {
        Some(one) => (one.table)(path, text),
        None => Table::plain(text),
    }
}

// The parser this path's language names, if it names one.
pub(crate) fn format(path: &Path) -> Option<Format> {
    owner(path)?.format
}

// The checker this path's language names, if it names one.
pub(crate) fn checker(path: &Path) -> Option<&'static Checker> {
    owner(path)?.check.as_ref()
}

// The test support this path's language names, if it names one.
pub(crate) fn tests(path: &Path) -> Option<&'static Tests> {
    owner(path)?.test.as_ref()
}

// The kind of piece this line opens, in its own language's word: `fn`, `impl`, `heading`, `fence`.
pub(crate) fn shape<'a>(table: &Table, line: &'a Line) -> Option<&'a str> {
    (table.language()?.shape)(line)
}
