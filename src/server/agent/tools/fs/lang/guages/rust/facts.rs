// Rust's facts for the module graph: units, declarations, exports, and the edges between files.

use std::path::{Path, PathBuf};

use crate::server::agent::tools::fs::lang::graph::{Decl, Domain, Loader, RawEdge, Unit};
use crate::server::agent::tools::fs::lang::{self, Hub, Table};
use crate::server::agent::tools::workspace::canonicalize_lenient;

pub(crate) static HUB: Hub = Hub {
    edges,
    unit_of: Some(unit_of),
    territory: Some(territory),
    interface: Some(interface),
    domain: Some(domain),
    loose: None,
    view: Some(super::view::view),
};

// Where a path belongs in its crate, as a unit. Directories answer too, so a parent can be found.
fn unit_of(path: &Path) -> Option<Unit> {
    let path = canonicalize_lenient(path);
    let root = crate_root(&path)?;
    let src = root.join("src");
    let rel = path.strip_prefix(&src).ok()?;
    if path.is_dir() {
        let chain = parts_of(rel);
        let entry = hub_of(&path, &src);
        return Some(Unit {
            name: chain.join("::"),
            place: path,
            entry,
        });
    }
    let stem = rel.file_stem()?.to_string_lossy().into_owned();
    let parent = rel.parent().unwrap_or_else(|| Path::new(""));
    let mut chain = parts_of(parent);
    let mut dir = src.join(parent);
    let root_hub = chain.is_empty() && (stem == "lib" || stem == "main");
    if stem != "mod" && !root_hub {
        // A dual-track hub: `x.rs` beside an `x/` directory owns the directory's files.
        let child = dir.join(&stem);
        if child.is_dir() {
            chain.push(stem.clone());
            dir = child;
        }
    }
    let mut entry = hub_of(&dir, &src);
    if entry.is_none() && std::fs::read_to_string(&path).is_ok_and(|text| declares_modules(&text)) {
        // A directory with no entry file whose file declares modules: that file is the entry.
        entry = Some(path.clone());
    }
    Some(Unit {
        name: chain.join("::"),
        place: dir,
        entry,
    })
}

// The search domain: the crate root, and the source tree under it. Directories answer too.
fn domain(path: &Path) -> Option<Domain> {
    let path = canonicalize_lenient(path);
    let root = crate_root(&path)?;
    let tree = root.join("src");
    tree.is_dir().then_some(Domain { root, tree })
}

// Where a unit's children are resolved: its directory, or the dual-track directory beside its file.
fn child_base(unit: &Unit) -> PathBuf {
    if unit.place.is_dir() {
        return unit.place.clone();
    }
    let Some(stem) = unit.place.file_stem() else {
        return unit.place.clone();
    };
    let beside = unit.place.with_file_name(stem);
    if beside.is_dir() {
        beside
    } else {
        unit.place.clone()
    }
}

// The units a unit declares, with the line and the visibility each is written with.
fn territory(unit: &Unit, loader: &Loader) -> Vec<Decl> {
    let text = unit
        .entry
        .as_ref()
        .and_then(|entry| loader.read(entry))
        .unwrap_or_default();
    let base = child_base(unit);
    let mut out = Vec::new();
    let mut declared = false;
    for (line, (name, vis)) in declarations(&text) {
        declared = true;
        let Some((path, dir)) = resolve_child(&base, &name) else {
            continue;
        };
        out.push(Decl {
            unit: Unit {
                name: name.clone(),
                place: if dir { base.join(&name) } else { path.clone() },
                entry: Some(path),
            },
            line: Some(line),
            from: unit.entry.clone(),
            tags: vec![vis],
        });
    }
    if declared {
        return out;
    }
    // No declarations: the directory's own files are the units, the entry excluded.
    for child in Loader::entries(&base) {
        if child.dir {
            continue;
        }
        let Some(stem) = child.name.strip_suffix(".rs") else {
            continue;
        };
        if stem == "mod" || Some(&child.path) == unit.entry.as_ref() {
            continue;
        }
        out.push(Decl {
            unit: Unit {
                name: stem.to_string(),
                place: child.path.clone(),
                entry: Some(child.path),
            },
            line: None,
            from: None,
            tags: vec!["?".to_string()],
        });
    }
    out
}

// What a file exposes: its `pub` items, then its `pub use`s, nothing dropped.
fn interface(path: &Path, text: &str) -> Vec<String> {
    let mut out = export_items(path, text);
    for item in re_exports(text) {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

// One file's `use` statements, each resolved to the files it lands on.
fn edges(path: &Path, text: &str, loader: &Loader) -> Vec<RawEdge> {
    let Some(root) = crate_root(path) else {
        return Vec::new();
    };
    let src = root.join("src");
    let Some(chain) = chain_of(path, &src) else {
        return Vec::new();
    };
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    // The forwarding table is only needed for a path that does not land directly.
    let mut table: Option<Vec<Forward>> = None;
    let mut out = Vec::new();
    for (line, paths, public) in uses(text) {
        for raw in paths {
            let wrote = raw.join("::");
            let Some(full) = written(&raw, &chain, dir) else {
                // A bare name with nothing beside it points outside the tree; keep it visible.
                out.push(RawEdge {
                    line: line - 1,
                    wrote,
                    symbols: raw.last().cloned().into_iter().collect(),
                    targets: Vec::new(),
                    via: Vec::new(),
                    re_export: public,
                });
                continue;
            };
            let (resolved, via) = if prefix_of(&full, &src).is_some() {
                (full.clone(), Vec::new())
            } else {
                let table = table.get_or_insert_with(|| forwards(path, &src, loader));
                trace(&full, table)
            };
            let mut targets = Vec::new();
            let mut k = resolved.len();
            while k > 0 {
                if let Some(files) = module_files(&resolved[..k], &src) {
                    targets = files;
                    break;
                }
                k -= 1;
            }
            out.push(RawEdge {
                line: line - 1,
                wrote,
                symbols: resolved.last().cloned().into_iter().collect(),
                targets,
                via,
                re_export: public,
            });
        }
    }
    out
}

// The directories a path is made of, as module names.
fn parts_of(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect()
}

fn crate_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|dir| dir.join("Cargo.toml").is_file())
        .map(Path::to_path_buf)
}

// A file's module chain from the crate root.
fn chain_of(file: &Path, src: &Path) -> Option<Vec<String>> {
    let rel = file.strip_prefix(src).ok()?;
    let stem = rel.file_stem()?.to_string_lossy().into_owned();
    let mut chain = rel.parent().map(parts_of).unwrap_or_default();
    if stem != "mod" && !(chain.is_empty() && (stem == "lib" || stem == "main")) {
        chain.push(stem);
    }
    Some(chain)
}

// The entry file of a module directory: `mod.rs`, the dual-track `x.rs`, or the crate root's.
fn hub_of(dir: &Path, src: &Path) -> Option<PathBuf> {
    if dir == src {
        let lib = src.join("lib.rs");
        if lib.is_file() {
            return Some(lib);
        }
        let main = src.join("main.rs");
        return main.is_file().then_some(main);
    }
    let module = dir.join("mod.rs");
    if module.is_file() {
        return Some(module);
    }
    let sibling = dir.with_extension("rs");
    sibling.is_file().then_some(sibling)
}

fn resolve_child(dir: &Path, name: &str) -> Option<(PathBuf, bool)> {
    let module = dir.join(name).join("mod.rs");
    if module.is_file() {
        return Some((module, true));
    }
    let file = dir.join(format!("{name}.rs"));
    file.is_file().then_some((file, false))
}

// Does this text declare a module that lives in its own file? An inline `mod x { … }` and a `#[cfg]`-hidden declaration both stay out: the first has no file, the second no answer.
pub(super) fn declares_modules(text: &str) -> bool {
    text.lines().any(|line| {
        let line = strip_vis(strip_attrs(line.trim_start()));
        line.strip_prefix("mod ").is_some_and(|rest| {
            let rest = rest.trim_end();
            rest.ends_with(';') && !rest.contains('{')
        })
    })
}

// `pub mod x;` lines: the 0-based line, the name, and the visibility as written.
fn declarations(text: &str) -> Vec<(usize, (String, String))> {
    let mut out = Vec::new();
    for (at, line) in text.lines().enumerate() {
        let raw = strip_attrs(line.trim_start());
        let vis = if raw.starts_with("pub") {
            let rest = strip_vis(raw);
            if rest.len() < raw.len() {
                let end = raw
                    .find(')')
                    .map_or_else(|| raw.find(' ').unwrap_or(raw.len()), |i| i + 1);
                raw[..end].trim().to_string()
            } else {
                "pub".to_string()
            }
        } else {
            "private".to_string()
        };
        let rest = strip_vis(raw);
        let Some(rest) = rest.strip_prefix("mod ") else {
            continue;
        };
        let rest = rest.trim_end();
        if !rest.ends_with(';') || rest.contains('{') {
            continue;
        }
        let name = rest.trim_end_matches(';').trim();
        if name.is_empty() || name.contains(' ') {
            continue;
        }
        out.push((at, (name.to_string(), vis)));
    }
    out
}

// A file's `pub` items, signature only: no bodies, no private items.
fn export_items(path: &Path, text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let table = lang::table(path, text);
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = strip_attrs(lines[i].trim_start());
        if !raw.starts_with("pub") || !exportable(&table, i) {
            i += 1;
            continue;
        }
        let mut buf = raw.to_string();
        let mut j = i;
        while !buf.contains('{') && !buf.contains(';') && j + 1 < lines.len() && j - i < 6 {
            j += 1;
            buf.push(' ');
            buf.push_str(lines[j].trim());
        }
        let cut = buf.find(['{', ';']).unwrap_or(buf.len());
        let signature = buf[..cut]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace("( ", "(")
            .replace(" )", ")")
            .replace(",)", ")");
        out.push(signature);
        i = j + 1;
    }
    out
}

fn exportable(table: &Table, at: usize) -> bool {
    table.line(at).is_some_and(|line| {
        line.head()
            && !matches!(
                lang::shape(table, line),
                None | Some("use" | "impl" | "mod")
            )
    })
}

// The `pub use` statements, each on one line.
fn re_exports(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = strip_attrs(lines[i].trim_start());
        if !raw.starts_with("pub") || !strip_vis(raw).starts_with("use ") {
            i += 1;
            continue;
        }
        let mut buf = raw.to_string();
        while !buf.contains(';') && i + 1 < lines.len() {
            i += 1;
            buf.push(' ');
            buf.push_str(lines[i].trim());
        }
        out.push(buf.split(';').next().unwrap_or("").trim().to_string());
        i += 1;
    }
    out
}

// Every `use` statement: its 1-based start line, the paths it names, and whether it forwards.
fn uses(text: &str) -> Vec<(usize, Vec<Vec<String>>, bool)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = strip_attrs(lines[i].trim_start());
        let public = raw.starts_with("pub");
        let stripped = strip_vis(raw);
        let Some(rest) = stripped.strip_prefix("use ") else {
            i += 1;
            continue;
        };
        let start = i;
        let mut buf = rest.trim().to_string();
        while !buf.contains(';') && i + 1 < lines.len() {
            i += 1;
            buf.push(' ');
            buf.push_str(lines[i].trim());
        }
        let body = buf.split(';').next().unwrap_or("");
        out.push((start + 1, expand(body), public));
        i += 1;
    }
    out
}

fn expand(tree: &str) -> Vec<Vec<String>> {
    expand_named(tree)
        .into_iter()
        .map(|(path, _)| path)
        .collect()
}

fn expand_named(tree: &str) -> Vec<(Vec<String>, String)> {
    let tree = tree.trim();
    if tree.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(Vec<String>, String)> = Vec::new();
    if let Some((head, inner)) = split_group(tree) {
        for part in split_top(inner) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some((nested, body)) = split_group(part) {
                let mut prefix = segments(head);
                prefix.extend(segments(nested));
                out.extend(expand_named(&format!("{}::{{{body}}}", prefix.join("::"))));
            } else {
                let mut prefix = segments(head);
                let (tail, name) = split_alias(part);
                prefix.extend(segments(tail));
                if named(&name) {
                    out.push((prefix, name));
                }
            }
        }
    } else {
        let (tail, name) = split_alias(tree);
        if named(&name) {
            out.push((segments(tail), name));
        }
    }
    out
}

fn named(name: &str) -> bool {
    !name.is_empty() && name != "*"
}

fn split_alias(text: &str) -> (&str, String) {
    let text = text.trim();
    if let Some((left, right)) = text.split_once(" as ") {
        (left.trim(), right.trim().to_string())
    } else {
        let name = text
            .rsplit("::")
            .find(|part| !part.is_empty())
            .unwrap_or("");
        (text, name.to_string())
    }
}

fn segments(text: &str) -> Vec<String> {
    let name = text.split(" as ").next().unwrap_or(text).trim();
    name.split("::")
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn split_group(tree: &str) -> Option<(&str, &str)> {
    let open = tree.find('{')?;
    let mut depth = 0;
    let mut close = None;
    for (at, ch) in tree.char_indices().skip(open) {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(at);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let head = tree[..open].trim_end_matches("::").trim();
    Some((head, &tree[open + 1..close]))
}

fn split_top(inner: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (at, ch) in inner.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&inner[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    out.push(&inner[start..]);
    out
}

// One forwarded name: where a consumer writes it, where it really lives, and who forwards it.
struct Forward {
    at: Vec<String>,
    to: Vec<String>,
    from: PathBuf,
}

// Every `pub use` on the way from this file up to the crate root.
fn forwards(path: &Path, src: &Path, loader: &Loader) -> Vec<Forward> {
    let mut out = Vec::new();
    let mut place = path.parent().map(Path::to_path_buf);
    while let Some(dir) = place {
        if !dir.starts_with(src) {
            break;
        }
        if let Some(hub) = hub_of(&dir, src)
            && let Some(chain) = chain_of(&hub, src)
            && let Some(text) = loader.read(&hub)
        {
            for (path, name) in pub_uses(&text) {
                let Some(to) = written(&path, &chain, &dir) else {
                    continue;
                };
                let mut at = chain.clone();
                at.push(name);
                out.push(Forward {
                    at,
                    to,
                    from: hub.clone(),
                });
            }
        }
        if dir == src {
            break;
        }
        place = dir.parent().map(Path::to_path_buf);
    }
    out
}

fn pub_uses(text: &str) -> Vec<(Vec<String>, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = strip_attrs(lines[i].trim_start());
        if !raw.starts_with("pub") || !strip_vis(raw).starts_with("use ") {
            i += 1;
            continue;
        }
        let mut buf = strip_vis(raw).trim_start().to_string();
        while !buf.contains(';') && i + 1 < lines.len() {
            i += 1;
            buf.push(' ');
            buf.push_str(lines[i].trim());
        }
        out.extend(expand_named(buf.split(';').next().unwrap_or("")));
        i += 1;
    }
    out
}

// A consumer's path as it really points, through any forwarding on the way.
fn trace(path: &[String], forwards: &[Forward]) -> (Vec<String>, Vec<PathBuf>) {
    let mut current = path.to_vec();
    let mut via = Vec::new();
    for _ in 0..8 {
        let mut found: Option<&Forward> = None;
        for forward in forwards {
            if current.len() < forward.at.len() || current[..forward.at.len()] != forward.at[..] {
                continue;
            }
            if found.is_none_or(|old| old.at.len() < forward.at.len()) {
                found = Some(forward);
            }
        }
        let Some(forward) = found else {
            break;
        };
        let mut next = forward.to.clone();
        next.extend(current[forward.at.len()..].iter().cloned());
        if next == current {
            break;
        }
        current = next;
        via.push(forward.from.clone());
    }
    (current, via)
}

// A use path as written inside `dir`: `crate`/`self`/`super`, or a module sitting beside it.
fn written(path: &[String], chain: &[String], dir: &Path) -> Option<Vec<String>> {
    if let Some(absolute) = absolute(path, chain) {
        return Some(absolute);
    }
    let first = path.first()?;
    let beside = dir.join(format!("{first}.rs")).is_file() || dir.join(first).is_dir();
    beside.then(|| {
        let mut base = chain.to_vec();
        base.extend(path.iter().cloned());
        base
    })
}

// A use path → the path from the crate root, when it names one.
fn absolute(path: &[String], chain: &[String]) -> Option<Vec<String>> {
    let (mut base, rest) = match path.first().map(String::as_str)? {
        "crate" => (Vec::new(), &path[1..]),
        "self" => (chain.to_vec(), &path[1..]),
        "super" => {
            let mut up = 0;
            while path.get(up).map(String::as_str) == Some("super") {
                up += 1;
            }
            if up > chain.len() {
                return None;
            }
            (chain[..chain.len() - up].to_vec(), &path[up..])
        }
        _ => return None,
    };
    base.extend(rest.iter().filter(|part| part.as_str() != "self").cloned());
    Some(base)
}

// The longest prefix of a path that names a module on disk.
fn prefix_of(path: &[String], src: &Path) -> Option<Vec<String>> {
    let mut k = path.len();
    while k > 0 {
        if module_files(&path[..k], src).is_some() {
            return Some(path[..k].to_vec());
        }
        k -= 1;
    }
    None
}

// The files a module path may land on, in the order a read tries them.
fn module_files(segs: &[String], src: &Path) -> Option<Vec<PathBuf>> {
    let base = segs
        .iter()
        .fold(src.to_path_buf(), |dir, part| dir.join(part));
    let mut out = Vec::new();
    let file = base.with_extension("rs");
    if file.is_file() {
        out.push(file);
    }
    let module = base.join("mod.rs");
    if module.is_file() {
        out.push(module);
    }
    (!out.is_empty()).then_some(out)
}

// The words before an item, dropped: `pub(crate) ` and consecutive `#[…]` hats.
fn strip_vis(line: &str) -> &str {
    let line = line.trim_start();
    let Some(rest) = line.strip_prefix("pub") else {
        return line;
    };
    let rest = rest.trim_start();
    if let Some(inner) = rest.strip_prefix('(')
        && let Some(end) = inner.find(')')
    {
        return inner[end + 1..].trim_start();
    }
    rest
}

pub(super) fn strip_attrs(mut line: &str) -> &str {
    loop {
        let line_trimmed = line.trim_start();
        if !line_trimmed.starts_with("#[") && !line_trimmed.starts_with("#![") {
            return line_trimmed;
        }
        match line_trimmed.find(']') {
            Some(end) => line = &line_trimmed[end + 1..],
            None => return line_trimmed,
        }
    }
}
