// CP's facts for the graph: one file is one unit, and every header it includes hangs under it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use super::scan::{self, Blank};
use crate::server::agent::tools::fs::lang::Hub;
use crate::server::agent::tools::fs::lang::Line;
use crate::server::agent::tools::fs::lang::graph::{Decl, Domain, Loader, RawEdge, Unit};
use crate::server::agent::tools::workspace::canonicalize_lenient;

pub(crate) static HUB: Hub = Hub {
    edges,
    unit_of: Some(unit_of),
    territory: Some(territory),
    interface: Some(interface),
    domain: Some(domain),
    loose: Some(loose),
    view: None,
};

// The files that name a project of this language.
pub(super) const MARKS: &[&str] = &[
    "compile_commands.json",
    "CMakeLists.txt",
    "Makefile",
    "meson.build",
];

// The files that describe a build, as opposed to one a build generated: a project is named after its build description.
pub(super) const BUILDS: &[&str] = &["CMakeLists.txt", "Makefile", "meson.build"];

// Names one interface line lists before the rest fold into a count.
const LISTED: usize = 12;

// How many entries of one compile database are read: a directory list beyond this is no longer this tool's business.
const ENTRIES: usize = 4096;

// Whether this path is one of this language's files.
pub(super) fn sorts(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| super::table::SUFFIXES.contains(&ext))
}

// The nearest directory above a path that names a project, then the repository it sits in, then its own directory.
fn project_root(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    let marked = start
        .ancestors()
        .find(|dir| MARKS.iter().any(|mark| dir.join(mark).is_file()))
        .map(Path::to_path_buf);
    marked
        .or_else(|| {
            start
                .ancestors()
                .find(|dir| dir.join(".git").exists())
                .map(Path::to_path_buf)
        })
        .or(Some(start))
}

// The search domain: the project root both ways. A C file is included by whoever is above it, and nothing tells a header which directory owns it.
fn domain(path: &Path) -> Option<Domain> {
    let path = canonicalize_lenient(path);
    let root = project_root(&path)?;
    Some(Domain {
        tree: root.clone(),
        root,
    })
}

// The tree one directory owns on its own: a directory holding CP files outright.
fn loose(dir: &Path) -> Option<PathBuf> {
    let holds = Loader::entries(dir)
        .into_iter()
        .any(|entry| !entry.dir && sorts(&entry.path));
    holds.then(|| dir.to_path_buf())
}

// Where a file belongs: one file is one unit, named by its path from the project root.
fn unit_of(path: &Path) -> Option<Unit> {
    let path = canonicalize_lenient(path);
    if !path.is_file() || !sorts(&path) {
        return None;
    }
    let root = project_root(&path)?;
    let name = path
        .strip_prefix(&root)
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    Some(Unit {
        name,
        place: path.clone(),
        entry: Some(path),
    })
}

// The files this one includes, each resolved to a file on disk.
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
        let tag = if super::table::source(target) {
            "source"
        } else {
            "header"
        };
        out.push(Decl {
            unit: child,
            line: Some(raw.line),
            from: Some(unit.place.clone()),
            tags: vec![tag.to_string()],
        });
    }
    out
}

// What a file offers the rest of the project: its declarations, one line each.
fn interface(path: &Path, text: &str) -> Vec<String> {
    let table = super::table::table(path, text);
    let mut out: Vec<String> = Vec::new();
    for (at, line) in table.pieces() {
        let Some(exact) = line.exact.as_deref() else {
            continue;
        };
        // A type's own members belong to the type; a namespace is a bucket of declarations, so what it holds is offered too.
        let scopes: Vec<&Line> = table
            .around(at)
            .into_iter()
            .filter(|(outer, _)| *outer != at)
            .map(|(_, outer)| outer)
            .collect();
        if scopes
            .iter()
            .any(|outer| !matches!(outer.label.as_deref(), Some("namespace" | "extern")))
        {
            continue;
        }
        if exact.starts_with("static ") || line.label.as_deref() == Some("extern") {
            continue;
        }
        let signature = exact.split_whitespace().collect::<Vec<_>>().join(" ");
        if !out.contains(&signature) {
            out.push(signature);
        }
    }
    if out.len() > LISTED {
        let more = out.len() - LISTED;
        out.truncate(LISTED);
        out.push(format!("[+{more} more]"));
    }
    out
}

// Every file this text includes, with the line it stands on and where it lands.
fn edges(path: &Path, text: &str, _loader: &Loader) -> Vec<RawEdge> {
    let dirs = include_dirs(path);
    let mut out: Vec<RawEdge> = Vec::new();
    for (line, wrote, angled) in includes(text) {
        let cut = wrote.split(['?', '#']).next().unwrap_or(&wrote);
        out.push(RawEdge {
            line,
            wrote: wrote.clone(),
            symbols: Vec::new(),
            targets: land(path, &dirs, cut, angled),
            via: Vec::new(),
            re_export: false,
        });
    }
    out
}

// Every `#include` a text holds: its 0-based line, the name it wrote, and whether it wrote it angled. A commented-out directive is not one, so the text is read with its comments blanked and its literals kept.
fn includes(text: &str) -> Vec<(usize, String, bool)> {
    let mut out: Vec<(usize, String, bool)> = Vec::new();
    let code = scan::strip(text, comments);
    for (at, line) in code.iter().enumerate() {
        let rest = line.trim_start();
        let Some(rest) = rest.strip_prefix('#') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix("include") else {
            continue;
        };
        let rest = rest.trim_start();
        let (close, angled) = match rest.chars().next() {
            Some('"') => ('"', false),
            Some('<') => ('>', true),
            _ => continue,
        };
        let body = &rest[1..];
        let Some(end) = body.find(close) else {
            continue;
        };
        let name = body[..end].trim();
        if name.is_empty() {
            continue;
        }
        out.push((at, name.to_string(), angled));
    }
    out
}

// The files an include lands on: quoted names start beside the including file, angled ones never do.
fn land(path: &Path, dirs: &[PathBuf], name: &str, angled: bool) -> Vec<PathBuf> {
    let mut tries: Vec<PathBuf> = Vec::new();
    if !angled && let Some(dir) = path.parent() {
        tries.push(dir.join(name));
    }
    for dir in dirs {
        tries.push(dir.join(name));
    }
    for try_at in tries {
        let found = canonicalize_lenient(&try_at);
        if found.is_file() {
            return vec![found];
        }
    }
    Vec::new()
}

// The directories a compiler was told to search for this file's project, from its compile database.
pub(super) fn include_dirs(path: &Path) -> Vec<PathBuf> {
    let Some(root) = project_root(path) else {
        return Vec::new();
    };
    static HELD: LazyLock<Mutex<HashMap<PathBuf, Vec<PathBuf>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    if let Some(held) = lock(&HELD).get(&root) {
        return held.clone();
    }
    let found = database(&root).map(|dir| dirs_of(&dir)).unwrap_or_default();
    lock(&HELD).insert(root, found.clone());
    found
}

// The directory holding this project's compile database: the root itself, its build directory, or the one directory under it that holds one.
pub(super) fn database(root: &Path) -> Option<PathBuf> {
    let own = root.join("compile_commands.json");
    if own.is_file() {
        return Some(root.to_path_buf());
    }
    let build = root.join("build").join("compile_commands.json");
    if build.is_file() {
        return Some(root.join("build"));
    }
    Loader::entries(root)
        .into_iter()
        .filter(|entry| entry.dir)
        .map(|entry| entry.path)
        .find(|dir| dir.join("compile_commands.json").is_file())
}

// The include directories one database names, in the order a compiler would meet them.
fn dirs_of(dir: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(dir.join("compile_commands.json")) else {
        return Vec::new();
    };
    let Ok(entries) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = Vec::new();
    for entry in entries.as_array().into_iter().flatten().take(ENTRIES) {
        let cwd = entry
            .get("directory")
            .and_then(|d| d.as_str())
            .map_or_else(|| dir.to_path_buf(), PathBuf::from);
        let args: Vec<String> = match entry.get("arguments").and_then(|a| a.as_array()) {
            Some(list) => list
                .iter()
                .filter_map(|one| one.as_str().map(str::to_string))
                .collect(),
            None => entry
                .get("command")
                .and_then(|c| c.as_str())
                .map(split)
                .unwrap_or_default(),
        };
        for (at, arg) in args.iter().enumerate() {
            let rest = if arg == "-I" || arg == "-isystem" {
                args.get(at + 1).map(String::as_str)
            } else {
                arg.strip_prefix("-I")
                    .map(|rest| rest.trim_start_matches('='))
            };
            let Some(rest) = rest.map(str::trim) else {
                continue;
            };
            if rest.is_empty() {
                continue;
            }
            let full = canonicalize_lenient(&cwd.join(rest));
            if full.is_dir() && !out.contains(&full) {
                out.push(full);
            }
        }
    }
    out
}

// A command line as a shell would split it, quotes and escapes behind it: the includes a compiler was handed are the only part read.
fn split(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    for c in command.chars() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => quote = Some(c),
            (None, ' ') => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
            }
            (None, c) => word.push(c),
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

// A lock that survives a panicking holder: the state behind it is a cache, never a promise.
fn lock<T>(held: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    held.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// The CP delimiters as an include scan needs them: comments blanked, literals and directives kept whole.
fn comments(blank: &mut Blank, c: char) -> bool {
    match c {
        '/' if blank.peek(1) == Some('/') => {
            blank.skip(2);
            blank.skip_to_eol();
            blank.gap();
        }
        '/' if blank.peek(1) == Some('*') => {
            blank.skip(2);
            blank.skip_block();
            blank.gap();
        }
        _ => return false,
    }
    true
}
