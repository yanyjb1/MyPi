// Python's facts for the module graph: the project domain, packages as units, their members, exports and imports.

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
    view: Some(super::view::view),
};

// How many `__init__.py` files one forwarded name is followed through.
const HOPS: usize = 3;

// A marker that names a project outright.
const STRONG: &[&str] = &["pyproject.toml", "setup.cfg", "setup.py"];

// A marker that hints at a project without claiming one: a stray requirement file must not outrank the project root above it.
const WEAK: &[&str] = &["requirements.txt"];

// The guard a type-only import sits behind.
const TYPING: &[&str] = &["if TYPE_CHECKING", "if typing.TYPE_CHECKING"];

// The nearest directory above a path that holds a marker of Python's own.
fn project_root(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() { path } else { path.parent()? };
    let marked = |marks: &[&str]| {
        start
            .ancestors()
            .find(|dir| marks.iter().any(|mark| dir.join(mark).is_file()))
            .map(Path::to_path_buf)
    };
    marked(STRONG)
        .or_else(|| marked(WEAK))
        .or_else(|| {
            start
                .ancestors()
                .find(|dir| dir.join(".git").exists())
                .map(Path::to_path_buf)
        })
        .or_else(|| Some(start.to_path_buf()))
}

// Whether a directory is a package: it holds an `__init__.py`.
pub(super) fn is_pkg(dir: &Path) -> bool {
    dir.join("__init__.py").is_file()
}

// Whether a directory is a package or holds packages of its own, so a parent knows it is not a leaf.
pub(super) fn has_pkg(dir: &Path) -> bool {
    Loader::entries(dir)
        .into_iter()
        .any(|entry| entry.dir && is_pkg(&entry.path))
}

// The directory a project's modules live under: `src` when it holds a package, the root otherwise.
fn tree_of(root: &Path) -> PathBuf {
    let src = root.join("src");
    if has_pkg(&src) {
        src
    } else {
        root.to_path_buf()
    }
}

// The search domain: the project root, and the tree its modules live under. The root stays the project's own, whatever the tree is: the upward walk has to reach the repository it sits in.
fn domain(path: &Path) -> Option<Domain> {
    let path = canonicalize_lenient(path);
    let root = project_root(&path)?;
    let tree = tree_of(&root);
    // A project with no marker of its own and no package under `src` is the loose scripts of one directory: its tree is that directory, its root the repository it sits in.
    let tree = if tree == root && !has_marker(&root) {
        loose_above(&path, &root).unwrap_or(tree)
    } else {
        tree
    };
    Some(Domain { tree, root })
}

// Whether this directory names a project of its own.
fn has_marker(dir: &Path) -> bool {
    STRONG
        .iter()
        .chain(WEAK)
        .any(|mark| dir.join(mark).is_file())
}

// The nearest directory at or above a path that holds Python files outright, up to the project root.
fn loose_above(path: &Path, root: &Path) -> Option<PathBuf> {
    let mut dir = path.parent();
    while let Some(at) = dir {
        if !at.starts_with(root) {
            break;
        }
        if loose(at).is_some() {
            return Some(at.to_path_buf());
        }
        if at == root {
            break;
        }
        dir = at.parent();
    }
    None
}

// The tree a directory owns on its own, when no marker stands in it: a directory holding Python files outright.
fn loose(dir: &Path) -> Option<PathBuf> {
    let holds = Loader::entries(dir)
        .into_iter()
        .any(|entry| !entry.dir && (entry.name.ends_with(".py") || entry.name.ends_with(".pyi")));
    holds.then(|| dir.to_path_buf())
}

// The dotted parts a path stands for: a directory's own name, or a module's whole chain with `__init__` dropped.
fn chain(path: &Path, tree: &Path) -> Option<Vec<String>> {
    let rel = path.strip_prefix(tree).ok()?;
    let mut parts: Vec<String> = rel
        .parent()
        .map(|dir| {
            dir.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    if path.is_dir() {
        if let Some(name) = path.file_name() {
            parts.push(name.to_string_lossy().into_owned());
        }
    } else {
        let stem = rel.file_stem()?.to_string_lossy().into_owned();
        if stem != "__init__" {
            parts.push(stem);
        }
    }
    Some(parts)
}

// Where a path belongs in its project, as a unit. Directories answer too, so a parent can be found.
fn unit_of(path: &Path) -> Option<Unit> {
    let path = canonicalize_lenient(path);
    let dom = domain(&path)?;
    let init = path.file_name().is_some_and(|name| name == "__init__.py");
    if !init && !path.is_file() {
        return None;
    }
    // An `__init__.py` is its package's entry, not a module of its own.
    let place = if init {
        path.parent()?.to_path_buf()
    } else {
        path.clone()
    };
    // A file outside the project's own tree, or a directory with no `__init__.py`, stands as its own unit: a test folder under a `src` layout deserves a module view like any other.
    let name = chain(&path, &dom.tree).map(|parts| parts.join("."));
    let name = match name {
        Some(name) if !name.is_empty() => name,
        _ => path.file_stem()?.to_string_lossy().into_owned(),
    };
    Some(Unit {
        name,
        place,
        entry: Some(path),
    })
}

// The units a package holds: its subpackages and its module files.
fn territory(unit: &Unit, _loader: &Loader) -> Vec<Decl> {
    if !unit.place.is_dir() {
        return Vec::new();
    }
    let Some(dom) = domain(&unit.place) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in Loader::entries(&unit.place) {
        if entry.name == "__init__.py" || entry.name == "__pycache__" {
            continue;
        }
        let (place, tag) = if entry.dir {
            if !is_pkg(&entry.path) {
                continue;
            }
            (entry.path.clone(), "pkg")
        } else if entry.name.ends_with(".py") || entry.name.ends_with(".pyi") {
            (entry.path.clone(), "mod")
        } else {
            continue;
        };
        let Some(parts) = chain(&place, &dom.tree) else {
            continue;
        };
        out.push(Decl {
            unit: Unit {
                name: parts.join("."),
                entry: Some(if entry.dir {
                    place.join("__init__.py")
                } else {
                    place.clone()
                }),
                place,
            },
            line: None,
            from: None,
            tags: vec![tag.to_string()],
        });
    }
    out
}

// What a file exposes: its `__all__` when one is written out, its public top-level names otherwise.
fn interface(path: &Path, text: &str) -> Vec<String> {
    let scanned = scan::strip(text);
    if let Some(names) = all_names(&scanned) {
        return names;
    }
    let mut out: Vec<String> = Vec::new();
    let init = path.file_name().is_some_and(|name| name == "__init__.py");
    for at in top_statements(&scanned) {
        let folded = fold(&scanned, at);
        if let Some((name, _)) = defined(&folded) {
            if !name.starts_with('_') {
                let line = signature(&folded);
                if !out.contains(&line) {
                    out.push(line);
                }
            }
            continue;
        }
        if init {
            continue;
        }
        if let Some(name) = constant(&folded)
            && !out.contains(&name)
        {
            out.push(name);
        }
    }
    if init {
        for name in forwarded(&scanned) {
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

// The names a top-level `__all__` lists, when its value is written out as string literals.
fn all_names(scanned: &scan::Scanned) -> Option<Vec<String>> {
    let at = top_statements(scanned).into_iter().find(|at| {
        let folded = fold(scanned, *at);
        folded.starts_with("__all__")
            && folded
                .trim_start_matches("__all__")
                .trim_start()
                .starts_with('=')
    })?;
    let body = fold(scanned, at).split_once('=')?.1.trim().to_string();
    if !(body.starts_with('[') || body.starts_with('(')) || !body.ends_with([']', ')']) {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    let mut rest = body.as_str();
    while let Some(at) = rest.find(['"', '\'']) {
        let quote = rest[at..].chars().next()?;
        let tail = &rest[at + quote.len_utf8()..];
        let end = tail.find(quote)?;
        let name = &tail[..end];
        out.push(name.to_string());
        rest = &tail[end + quote.len_utf8()..];
        if rest.contains(['+', '*', '[']) {
            // A name built by other means is not a literal list this reads.
            return None;
        }
    }
    (!out.is_empty()).then_some(out)
}

// Every line at the file's own top level, outside a folded run.
fn top_statements(scanned: &scan::Scanned) -> Vec<usize> {
    (0..scanned.code.len())
        .filter(|at| {
            !scanned.carried.get(*at).copied().unwrap_or(false)
                && !scanned.code[*at].trim().is_empty()
                && scanned.code[*at].len() == scanned.code[*at].trim_start().len()
        })
        .collect()
}

// The top-level statements, and the imports an `if TYPE_CHECKING:` block holds.
fn imports(scanned: &scan::Scanned) -> Vec<usize> {
    let mut out = top_statements(scanned);
    let mut guard = false;
    for at in 0..scanned.code.len() {
        let code = scanned.code[at].trim_start();
        if scanned.carried.get(at).copied().unwrap_or(false) || code.trim().is_empty() {
            continue;
        }
        let depth = scanned.code[at].len() - code.len();
        if depth == 0 {
            guard = TYPING.iter().any(|head| code.starts_with(head));
            continue;
        }
        if guard && (code.starts_with("import ") || code.starts_with("from ")) {
            out.push(at);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

// One statement's own text, folded over the lines it continues onto.
fn fold(scanned: &scan::Scanned, at: usize) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let mut line = at;
    loop {
        parts.push(scanned.code[line].trim());
        let next = line + 1;
        if scanned.carried.get(next).copied() != Some(true) {
            break;
        }
        line = next;
    }
    parts.join(" ")
}

// The name a `def` or a `class` binds, and the word that opens it.
fn defined(folded: &str) -> Option<(String, &'static str)> {
    let folded = folded.trim();
    let (rest, kind) = folded
        .strip_prefix("async def ")
        .map(|rest| (rest, "async def"))
        .or_else(|| folded.strip_prefix("def ").map(|rest| (rest, "def")))
        .or_else(|| folded.strip_prefix("class ").map(|rest| (rest, "class")))?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some((name, kind))
}

// One definition's header as the interface prints it: its own words, the block's colon dropped.
fn signature(folded: &str) -> String {
    folded
        .trim()
        .trim_end_matches(':')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// The name of a top-level constant: a word in capitals, assigned.
fn constant(folded: &str) -> Option<String> {
    let (left, right) = folded.split_once('=')?;
    if right.starts_with('=') {
        return None;
    }
    let name = left.trim();
    if name.is_empty() || name.contains([' ', ':', '.']) {
        return None;
    }
    let mut letters = name.chars().filter(char::is_ascii_alphabetic);
    let first = letters.next()?;
    let caps = first.is_ascii_uppercase() && letters.all(|c| c.is_ascii_uppercase());
    caps.then(|| name.to_string())
}

// The public names an `__init__.py` forwards with a top-level `from … import …`.
fn forwarded(scanned: &scan::Scanned) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for at in top_statements(scanned) {
        let folded = fold(scanned, at);
        if !folded.starts_with("from ") {
            continue;
        }
        for name in imported(&folded) {
            if !name.starts_with('_') && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

// The names a `from … import …` brings in, under the names they are bound to.
fn imported(folded: &str) -> Vec<String> {
    let Some((_, tail)) = folded.split_once(" import ") else {
        return Vec::new();
    };
    let tail = tail.trim();
    let inner = tail
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(tail);
    let mut out = Vec::new();
    for item in inner.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let name = item
            .rsplit_once(" as ")
            .map_or(item, |(_, alias)| alias.trim());
        if name.is_empty() || name == "*" {
            continue;
        }
        out.push(name.to_string());
    }
    out
}

// One file's imports: each name it brings in, resolved to the files it may land on.
fn edges(path: &Path, text: &str, loader: &Loader) -> Vec<RawEdge> {
    let Some(dom) = domain(path) else {
        return Vec::new();
    };
    let scanned = scan::strip(text);
    let init = path.file_name().is_some_and(|name| name == "__init__.py");
    let listed = all_names(&scanned).unwrap_or_default();
    // A name is resolved against the search roots Python itself would use: a package's file searches from the project tree, a loose script from its own folder upward.
    let roots = search_roots(path, &dom);
    let mut out = Vec::new();
    for at in imports(&scanned) {
        let folded = fold(&scanned, at);
        let Some((level, module, names)) = use_of(&folded) else {
            continue;
        };
        for (name, wrote) in names {
            let mut targets: Vec<PathBuf> = Vec::new();
            let mut via: Vec<PathBuf> = Vec::new();
            if level == 0 {
                for root in &roots {
                    let found =
                        resolve(root, &module, name.as_deref(), &dom.tree, loader, &mut via);
                    if !found.is_empty() {
                        targets = found;
                        break;
                    }
                }
            } else if let Ok(package) = package_of(path, &dom.tree)
                && let Some(base) = absolute(&package, level, &module)
            {
                targets = resolve(
                    &dom.tree,
                    &base,
                    name.as_deref(),
                    &dom.tree,
                    loader,
                    &mut via,
                );
            }
            let forwards = init || name.as_ref().is_some_and(|name| listed.contains(name));
            out.push(RawEdge {
                line: at,
                wrote,
                symbols: name.into_iter().collect(),
                targets,
                via,
                re_export: forwards,
            });
        }
    }
    out
}

// The roots an absolute import is tried against, in the order Python searches them. A file inside a package searches from the project tree alone; a loose script searches from its own folder upward to that tree.
fn search_roots(path: &Path, dom: &Domain) -> Vec<PathBuf> {
    let inside = path
        .parent()
        .is_some_and(|dir| start_of_package(dir).is_some());
    if inside {
        return vec![dom.tree.clone()];
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut dir = path.parent();
    while let Some(at) = dir {
        roots.push(at.to_path_buf());
        if at == dom.tree || !at.starts_with(&dom.tree) {
            break;
        }
        dir = at.parent();
    }
    if !roots.iter().any(|one| one == &dom.tree) {
        roots.push(dom.tree.clone());
    }
    roots
}

// The topmost directory of the package this file's own directory belongs to.
fn start_of_package(dir: &Path) -> Option<PathBuf> {
    let mut top = None;
    let mut at = Some(dir);
    while let Some(here) = at {
        if !is_pkg(here) {
            break;
        }
        top = Some(here.to_path_buf());
        at = here.parent();
    }
    top
}

// The files one module path lands on from a search root, following a forwarded name a few hops.
fn resolve(
    root: &Path,
    base: &[String],
    symbol: Option<&str>,
    tree: &Path,
    loader: &Loader,
    via: &mut Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut targets = match symbol {
        Some(name) => submodule(base, name, root).unwrap_or_else(|| landing(base, root)),
        None => landing(base, root),
    };
    if let Some(symbol) = symbol {
        let mut reached: Vec<PathBuf> = Vec::new();
        for file in &targets {
            if let Some(land) = follow(tree, file, symbol, loader, via)
                && !reached.contains(&land)
            {
                reached.push(land);
            }
        }
        if !reached.is_empty() {
            targets = reached;
        }
    }
    targets
}

// A statement's own reading: its relative depth, its module path, and one entry per name it binds.
type Use = (usize, Vec<String>, Vec<(Option<String>, String)>);

fn use_of(folded: &str) -> Option<Use> {
    let folded = folded.trim();
    if let Some(rest) = folded.strip_prefix("import ") {
        let mut out = Vec::new();
        for item in rest.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            let (path, alias) = match item.rsplit_once(" as ") {
                Some((path, alias)) => (path.trim(), Some(alias.trim().to_string())),
                None => (item, None),
            };
            let segs: Vec<String> = path.split('.').map(str::to_string).collect();
            if segs.iter().any(String::is_empty) {
                continue;
            }
            // `import a.b` binds `a`, not `b`: the dotted name is what the statement names.
            let bound = alias.or_else(|| segs.first().cloned());
            out.push((bound, path.to_string()));
        }
        return Some((0, Vec::new(), out));
    }
    let rest = folded.strip_prefix("from ")?;
    let (head, tail) = rest.split_once(" import ")?;
    let head = head.trim();
    let level = head.chars().take_while(|c| *c == '.').count();
    let module: Vec<String> = head
        .trim_start_matches('.')
        .split('.')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();
    let inner = tail.trim();
    let inner = inner
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(inner);
    let mut out = Vec::new();
    for item in inner.split(',') {
        let item = item.trim();
        if item.is_empty() || item == "*" {
            continue;
        }
        let name = item
            .rsplit_once(" as ")
            .map_or(item, |(_, alias)| alias.trim());
        if name.is_empty() || name == "*" {
            continue;
        }
        out.push((Some(name.to_string()), head.to_string()));
    }
    Some((level, module, out))
}

// The package a file belongs to, from the tree root.
fn package_of(path: &Path, tree: &Path) -> std::result::Result<Vec<String>, ()> {
    let rel = path.strip_prefix(tree).map_err(|_| ())?;
    Ok(rel
        .parent()
        .map(|dir| {
            dir.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default())
}

// A written module path as the path from the tree root; `None` when it climbs past the root.
fn absolute(package: &[String], level: usize, module: &[String]) -> Option<Vec<String>> {
    if level == 0 {
        return Some(module.to_vec());
    }
    let up = level - 1;
    if up > package.len() {
        return None;
    }
    let mut base = package[..package.len() - up].to_vec();
    base.extend(module.iter().cloned());
    Some(base)
}

// The files a module path may land on: a module, a stub, or the package's own entry.
fn landing(base: &[String], tree: &Path) -> Vec<PathBuf> {
    let at = base
        .iter()
        .fold(tree.to_path_buf(), |dir, part| dir.join(part));
    let mut out = Vec::new();
    for file in [at.with_extension("py"), at.with_extension("pyi")] {
        if file.is_file() {
            out.push(file);
        }
    }
    let entry = at.join("__init__.py");
    if entry.is_file() {
        out.push(entry);
    }
    out
}

// The files a submodule of one module path lands on, when the named thing is a module and not a name.
fn submodule(base: &[String], name: &str, tree: &Path) -> Option<Vec<PathBuf>> {
    let mut segs = base.to_vec();
    segs.push(name.to_string());
    let found = landing(&segs, tree);
    (!found.is_empty()).then_some(found)
}

// Follow one name through the `__init__.py` files that forward it, up to a few hops.
fn follow(
    tree: &Path,
    file: &Path,
    symbol: &str,
    loader: &Loader,
    via: &mut Vec<PathBuf>,
) -> Option<PathBuf> {
    if !file.file_name().is_some_and(|name| name == "__init__.py") || via.len() >= HOPS {
        return Some(file.to_path_buf());
    }
    let Ok(package) = package_of(file, tree) else {
        return Some(file.to_path_buf());
    };
    let text = loader.read(file)?;
    let scanned = scan::strip(&text);
    for at in top_statements(&scanned) {
        let folded = fold(&scanned, at);
        if !folded.starts_with("from ") {
            continue;
        }
        let Some((level, module, names)) = use_of(&folded) else {
            continue;
        };
        if !names
            .iter()
            .any(|(name, _)| name.as_deref() == Some(symbol))
        {
            continue;
        }
        let Some(base) = absolute(&package, level, &module) else {
            continue;
        };
        let targets = submodule(&base, symbol, tree).unwrap_or_else(|| landing(&base, tree));
        let Some(land) = targets.into_iter().find(|path| path != file) else {
            continue;
        };
        via.push(file.to_path_buf());
        return follow(tree, &land, symbol, loader, via);
    }
    Some(file.to_path_buf())
}
