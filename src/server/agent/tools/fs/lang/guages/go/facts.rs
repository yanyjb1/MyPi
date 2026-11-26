// Go's facts for the graph: one package is one unit, named by its import path, and every import it writes hangs under it.

use std::path::{Path, PathBuf};

use super::scan;
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
    view: None,
};

// The tree one directory owns on its own: a directory holding Go files outright.
fn loose(dir: &Path) -> Option<PathBuf> {
    let holds = Loader::entries(dir)
        .into_iter()
        .any(|entry| !entry.dir && sorts(&entry.path));
    holds.then(|| dir.to_path_buf())
}

// Whether this path is one of this language's files.
pub(super) fn sorts(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "go")
}

// The module a path belongs to: the nearest directory above it holding a module file. A Go file outside a module is one the toolchain itself refuses to build, so no unit claims it.
pub(super) fn module_root(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    start
        .ancestors()
        .find(|dir| dir.join("go.mod").is_file())
        .map(Path::to_path_buf)
}

// The search domain: the module root both ways. Go resolves every import from the module's own path, above all its directories alike.
fn domain(path: &Path) -> Option<Domain> {
    let path = canonicalize_lenient(path);
    let root = module_root(&path)?;
    Some(Domain {
        tree: root.clone(),
        root,
    })
}

// Where a path belongs: one package is one unit, its files sharing the directory that holds them. A directory answers too, so a parent can be found.
fn unit_of(path: &Path) -> Option<Unit> {
    let path = canonicalize_lenient(path);
    let root = module_root(&path)?;
    let place = if path.is_dir() {
        path.clone()
    } else {
        if !sorts(&path) {
            return None;
        }
        path.parent()?.to_path_buf()
    };
    let rel = place.strip_prefix(&root).ok()?;
    Some(Unit {
        name: import_path(&root, rel),
        // A package's own text is every file it holds, so a unit names its directory and leaves the entry empty.
        place,
        entry: None,
    })
}

// The import path a package is known by: the module's own path, then the directory's parts.
fn import_path(root: &Path, rel: &Path) -> String {
    let module = std::fs::read_to_string(root.join("go.mod"))
        .ok()
        .and_then(|text| module_path(&text))
        .unwrap_or_default();
    let mut parts: Vec<String> = rel
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .filter(|part| !part.is_empty() && part != ".")
        .collect();
    if module.is_empty() {
        return parts.join("/");
    }
    if parts.is_empty() {
        return module;
    }
    parts.insert(0, module);
    parts.join("/")
}

// The module path a `go.mod` declares.
pub(super) fn module_path(manifest: &str) -> Option<String> {
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with("//") {
            continue;
        }
        let Some(rest) = line.strip_prefix("module") else {
            continue;
        };
        let rest = rest.trim_start();
        if rest.is_empty() {
            continue;
        }
        let value = rest.trim().trim_matches('"').trim_matches('`');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

// The packages a directory declares: every subdirectory holding a Go file, and the directory itself when it holds one. Go has no module statement to point at, so a child's line is its own directory entry and carries none.
fn territory(unit: &Unit, _loader: &Loader) -> Vec<Decl> {
    let mut out: Vec<Decl> = Vec::new();
    for entry in Loader::entries(&unit.place) {
        if !entry.dir {
            continue;
        }
        let Some(child) = unit_of(&entry.path) else {
            continue;
        };
        let holds = Loader::entries(&entry.path)
            .into_iter()
            .any(|file| !file.dir && sorts(&file.path));
        if !holds {
            continue;
        }
        // Go declares no child anywhere: a package is known by where it sits, so its declaration points at no line.
        out.push(Decl {
            unit: child,
            line: Some(0),
            from: None,
            tags: vec!["package".to_string()],
        });
    }
    out
}

// What one file exposes: every name it declares with a capital letter, which is what Go calls exported. A group binds no name of its own, so its frame is skipped and the rows inside it carry both the name and the kind.
fn interface(_path: &Path, text: &str) -> Vec<String> {
    let table = super::table::table(Path::new("x.go"), text);
    let raw: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    for (at, line) in table.pieces() {
        let Some(kind) = crate::server::agent::tools::fs::lang::shape(&table, line) else {
            continue;
        };
        let Some(exact) = line.exact.as_deref() else {
            continue;
        };
        if exact.trim() == kind || kind == "package" {
            continue;
        }
        let declared = match raw.get(at) {
            Some(text) => exported(text.trim(), kind),
            None => None,
        };
        if let Some(declared) = declared
            && !out.contains(&declared)
        {
            out.push(declared);
        }
    }
    out
}

// The exported name one declaration binds, in Go's own words: `func Foo(` reads `Foo func`, `type Bar struct` reads `Bar type`, and a group's row `MaxSize = 10` reads `MaxSize const`.
fn exported(line: &str, kind: &str) -> Option<String> {
    let rest = line
        .strip_prefix(kind)
        .map_or(line, str::trim_start)
        .trim_start();
    // A method's own receiver stands between the head word and its name.
    let rest = if kind == "func" && rest.starts_with('(') {
        rest.split_once(')')?.1.trim_start()
    } else {
        rest
    };
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    name.chars()
        .next()
        .is_some_and(char::is_uppercase)
        .then(|| format!("{name} {kind}"))
}

// One file's imports, each resolved to the directory of the package it lands on.
fn edges(path: &Path, text: &str, _loader: &Loader) -> Vec<RawEdge> {
    let Some(root) = module_root(path) else {
        return Vec::new();
    };
    let Ok(module) = std::fs::read_to_string(root.join("go.mod")) else {
        return Vec::new();
    };
    let Some(module) = module_path(&module) else {
        return Vec::new();
    };
    let raw: Vec<&str> = text.lines().collect();
    let code = scan::strip(text, super::table::token);
    let mut out = Vec::new();
    for (line, wrote) in imports(&code, &raw) {
        // A name the module's own path begins with lands inside this tree; anything else is outside it, and stays visible as written.
        let rest = if wrote == module {
            ""
        } else {
            match wrote
                .strip_prefix(&module)
                .and_then(|tail| tail.strip_prefix('/'))
            {
                Some(rest) => rest,
                None => {
                    out.push(RawEdge {
                        symbols: leaf_of(&wrote),
                        line,
                        wrote,
                        targets: Vec::new(),
                        via: Vec::new(),
                        re_export: false,
                    });
                    continue;
                }
            }
        };
        let target = root.join(rest);
        out.push(RawEdge {
            line,
            symbols: leaf_of(&wrote),
            wrote,
            targets: entry_file(&target).into_iter().collect(),
            via: Vec::new(),
            re_export: false,
        });
    }
    out
}

// Every import a file writes, as the 0-based line it stands on and the path it names: `import "fmt"`, an `import (` group with one row each, and a row with an alias in front.
pub(super) fn imports(code: &[String], raw: &[&str]) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut inside = false;
    for (at, line) in code.iter().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("import") {
            let rest = rest.trim_start();
            if rest.starts_with('(') {
                inside = true;
                continue;
            }
            // The blanked text says where the import is; the file's own line holds the path the scan cut out.
            if let Some(wrote) = quoted(raw.get(at).copied().unwrap_or("")) {
                out.push((at, wrote));
            }
            continue;
        }
        if !inside {
            continue;
        }
        if trimmed.starts_with(')') {
            inside = false;
            continue;
        }
        if let Some(wrote) = quoted(raw.get(at).copied().unwrap_or("")) {
            out.push((at, wrote));
        }
    }
    out
}

// The name an import is called by where it is written: the last segment of its path.
fn leaf_of(wrote: &str) -> Vec<String> {
    wrote
        .rsplit('/')
        .next()
        .filter(|leaf| !leaf.is_empty())
        .map(str::to_string)
        .into_iter()
        .collect()
}

// The file a package lands on: an import names a directory, and the graph's own edges land on the file that speaks for it. A directory no source file holds answers nothing.
fn entry_file(dir: &Path) -> Option<PathBuf> {
    Loader::entries(dir)
        .into_iter()
        .find(|entry| !entry.dir && sorts(&entry.path) && !entry.name.ends_with("_test.go"))
        .map(|entry| entry.path)
}

// The path a quoted literal stands for, once an alias or a trailing comment is behind it.
fn quoted(line: &str) -> Option<String> {
    let start = line.find('"')?;
    let rest = &line[start + 1..];
    let end = rest.find('"')?;
    let wrote = rest[..end].trim();
    (!wrote.is_empty()).then(|| wrote.to_string())
}
