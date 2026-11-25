// The walk: the source trees a language claims, the directories a call narrows them to, and the matching lines.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use crate::server::agent::tools::fs::lang::graph::loader;
use crate::server::agent::tools::fs::lang::{self, Table};
use crate::server::agent::tools::workspace::canonicalize_lenient;

// Whether a sorter claims this file: `grep` reads what the languages read.
fn readable(path: &Path) -> bool {
    lang::owner(path).is_some()
}

// Threads one answer may read its files on.
const THREADS: usize = 8;

// Where the tree stands: the workspace root, and every source tree a language claims under it.
pub(crate) struct Zone {
    pub(crate) root: PathBuf,
    pub(crate) trees: Vec<PathBuf>,
}

// The trees the session stands among.
pub(crate) fn zone(cwd: &Path) -> Zone {
    let root = canonicalize_lenient(cwd);
    let trees = lang::trees(&root);
    Zone { root, trees }
}

// The files and directories to read, and one line per scope entry that could not be used.
pub(crate) fn roots(
    zone: &Zone,
    scope: &mut Vec<String>,
) -> (Vec<PathBuf>, Vec<PathBuf>, Vec<String>) {
    if scope.is_empty() {
        return (zone.trees.clone(), Vec::new(), Vec::new());
    }
    let trees: Vec<PathBuf> = zone
        .trees
        .iter()
        .map(|tree| canonicalize_lenient(tree))
        .collect();
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut misses: Vec<String> = Vec::new();
    // One entry per path, however many times the call named it.
    let mut named: Vec<String> = Vec::new();
    scope.retain(|entry| {
        let repeated = named.contains(entry);
        named.push(entry.clone());
        !repeated
    });

    for entry in scope {
        let raw = entry.trim();
        if raw.is_empty() {
            misses.push("[scope] (empty): no path".to_string());
            continue;
        }
        let named = Path::new(raw);
        let joined = if named.is_absolute() {
            named.to_path_buf()
        } else {
            zone.root.join(named)
        };
        let real = canonicalize_lenient(&joined);
        if !real.exists() {
            misses.push(format!("[scope] {raw}: does not exist"));
        } else if !trees.iter().any(|tree| real.starts_with(tree)) {
            misses.push(format!("[scope] {raw}: is outside the source tree"));
        } else if real.is_dir() {
            if !dirs.contains(&real) {
                dirs.push(real);
            }
        } else if readable(&real) {
            if !files.contains(&real) {
                files.push(real);
            }
        } else {
            misses.push(format!("[scope] {raw}: no sorter reads this file"));
        }
    }
    (dirs, files, misses)
}

// What the call looks for.
pub(crate) enum Matcher {
    Text(String),
    Pattern(regex::Regex),
}

impl Matcher {
    pub(crate) fn text(query: &str) -> Self {
        Self::Text(query.to_string())
    }

    // A pattern as written; its own syntax errors are the model's to fix.
    pub(crate) fn pattern(query: &str) -> Result<Self> {
        regex::Regex::new(query).map(Self::Pattern).map_err(|e| {
            anyhow!("`query` is not a valid pattern: {e}; leave `regex` off to search it as text")
        })
    }

    fn hits(&self, line: &str) -> bool {
        match self {
            Self::Text(needle) => line.contains(needle.as_str()),
            Self::Pattern(pattern) => pattern.is_match(line),
        }
    }
}

// One file the walk reads, the items its hits make, and its pieces.
pub(crate) struct File {
    pub(crate) path: PathBuf,
    pub(crate) text: String,
    pub(crate) items: Vec<Item>,
    pub(crate) table: Table,
}

// One drawn unit: the smallest piece holding a run of nested hits, and those hits.
pub(crate) struct Item {
    pub(crate) open: usize,
    pub(crate) close: usize,
    // Hit lines, 0-based, ascending.
    pub(crate) hits: Vec<usize>,
}

// The items a file's hits make: each hit takes its nearest enclosing piece, a span that lands inside an item joins it, and a later span that swallows items absorbs them into one.
fn items(table: &Table, hits: &[usize]) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for &line in hits {
        let (open, close) = match table.innermost(line) {
            Some((open, piece)) => (open, piece.mate.unwrap_or(open)),
            None => (line, line),
        };
        let mut held: Vec<usize> = Vec::new();
        while out
            .last()
            .is_some_and(|last| open <= last.open && close >= last.close)
        {
            let last = out.pop().expect("the guard just read it");
            held.extend(last.hits);
        }
        if held.is_empty()
            && let Some(item) = out.last_mut()
            && open >= item.open
            && close <= item.close
        {
            item.hits.push(line);
            continue;
        }
        held.push(line);
        held.sort_unstable();
        out.push(Item {
            open,
            close,
            hits: held,
        });
    }
    out
}

// Every named file, and every readable file under these directories; read in parallel.
pub(crate) fn collect(dirs: &[PathBuf], files: &[PathBuf], matcher: &Matcher) -> Vec<File> {
    let mut named: BTreeSet<PathBuf> = files.iter().cloned().collect();
    for dir in dirs {
        named.extend(lang::files(dir));
    }
    let paths: Vec<PathBuf> = named.into_iter().collect();

    if paths.len() <= 1 {
        return paths
            .iter()
            .filter_map(|path| read(path, matcher))
            .collect();
    }
    let batch = paths.len().div_ceil(THREADS).max(1);
    let mut out: Vec<File> = Vec::with_capacity(paths.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = paths
            .chunks(batch)
            .map(|chunk| scope.spawn(move || read_all(chunk, matcher)))
            .collect();
        for handle in handles {
            out.extend(handle.join().unwrap_or_default());
        }
    });
    out
}

fn read_all(paths: &[PathBuf], matcher: &Matcher) -> Vec<File> {
    paths
        .iter()
        .filter_map(|path| read(path, matcher))
        .collect()
}

// One file's hits and its table; a file that cannot be read is simply not there.
fn read(path: &Path, matcher: &Matcher) -> Option<File> {
    let text = loader::text(path)?;
    let hits: Vec<usize> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| matcher.hits(line))
        .map(|(at, _)| at)
        .collect();
    let table = lang::table(path, &text);
    let items = items(&table, &hits);
    Some(File {
        path: path.to_path_buf(),
        text,
        items,
        table,
    })
}
