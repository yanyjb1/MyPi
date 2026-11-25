// The Python relations: `A.b` as a class member or a module member, and a name's own declarations.

use std::collections::BTreeSet;
use std::path::Path;

use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::{self, Line, Relation, Relations, Row, Source, Table};

// How Python's relations read: a module or a class, named by a dot.
pub(crate) const RELATIONS: Relations = Relations { scan };

// Members one block lists.
const MEMBERS: usize = 12;

// The rows a query answers: `A.b` a class or module member, a lone name its declarations.
fn scan(sources: &[Source], root: &Path, query: &str) -> Relation {
    let mut used = Vec::new();
    if let Some((a, b)) = two_segments(query) {
        let mut rows = Vec::new();
        let mine = class_rows(sources, root, &a, &b, MEMBERS, &mut used);
        if mine.is_empty() {
            rows.push(Row {
                heading: None,
                text: format!("[class] {a}.{b}: no class `{a}` in this project declares `{b}`"),
            });
        } else {
            rows.push(Row {
                heading: Some(format!("[class] {a}.{b}")),
                text: String::new(),
            });
            rows.extend(mine);
        }
        rows.push(Row {
            heading: None,
            text: String::new(),
        });
        let (module, known) = module_rows(sources, root, &a, &b, &mut used);
        if module.is_empty() {
            let text = if known {
                format!("[module] {a}.{b}: module `{a}` declares no `{b}`")
            } else {
                format!("[module] {a}.{b}: no module `{a}` in this project")
            };
            rows.push(Row {
                heading: None,
                text,
            });
        } else {
            rows.push(Row {
                heading: Some(format!("[module] {a}.{b}")),
                text: String::new(),
            });
            rows.extend(module);
        }
        return Relation { rows, used };
    }
    if let Some(name) = single_name(query) {
        let rows = x_ray_rows(sources, root, name, MEMBERS, &mut used);
        if rows.is_empty() {
            return Relation {
                rows: Vec::new(),
                used: Vec::new(),
            };
        }
        return Relation { rows, used };
    }
    Relation {
        rows: Vec::new(),
        used: Vec::new(),
    }
}

// `A.b` where `b` is declared inside the class `A`: its methods and its own assignments.
fn class_rows(
    sources: &[Source],
    root: &Path,
    a: &str,
    b: &str,
    items: usize,
    used: &mut Vec<usize>,
) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for (at, src) in sources.iter().enumerate() {
        if !declares_class(src.table, a) {
            continue;
        }
        let raw: Vec<&str> = src.text.lines().collect();
        let label = parts::rel(root, src.path);
        for (index, line) in src.table.pieces() {
            if lang::shape(src.table, line) != Some("class") || !names(src.table, line, a) {
                continue;
            }
            let end = src.table.end(index).unwrap_or(index);
            let mut mine: Vec<String> = Vec::new();
            for (member_at, member) in src.table.inner(index) {
                if names(src.table, member, b) {
                    mine.push(member_row(src.table, member_at));
                }
            }
            for line in members(&raw, index + 1, end, b) {
                let text = raw[line].trim();
                if !mine.iter().any(|held| held.contains(text)) {
                    mine.push(format!("{}  {}", line + 1, text));
                }
            }
            if mine.is_empty() {
                continue;
            }
            used.push(at);
            rows.push(Row {
                heading: None,
                text: format!("  {label}:{}  {}", index + 1, exact(line)),
            });
            for row in mine.iter().take(items) {
                rows.push(Row {
                    heading: None,
                    text: format!("    {row}"),
                });
            }
            if mine.len() > items {
                rows.push(Row {
                    heading: None,
                    text: format!("    [+{} more]", mine.len() - items),
                });
            }
        }
    }
    rows
}

// `A.b` where `A` is a module: the item `b` that module declares.
fn module_rows(
    sources: &[Source],
    root: &Path,
    a: &str,
    b: &str,
    used: &mut Vec<usize>,
) -> (Vec<Row>, bool) {
    let mut rows: Vec<Row> = Vec::new();
    let mut known = false;
    let mut seen: BTreeSet<(String, usize)> = BTreeSet::new();
    for (at, src) in sources.iter().enumerate() {
        if !answers(src.path, a) {
            continue;
        }
        known = true;
        let label = parts::rel(root, src.path);
        for (item_at, item) in src.table.pieces() {
            if !names(src.table, item, b) || lang::shape(src.table, item) == Some("import") {
                continue;
            }
            if !seen.insert((label.clone(), item_at)) {
                continue;
            }
            used.push(at);
            rows.push(Row {
                heading: None,
                text: format!(
                    "  {label}:{}  {}  ({}..{})",
                    item_at + 1,
                    exact(item),
                    item_at + 1,
                    src.table.end(item_at).unwrap_or(item_at) + 1
                ),
            });
        }
        // A top-level constant is no piece of its own: its own line is the whole of it.
        let raw: Vec<&str> = src.text.lines().collect();
        for (line_at, text) in raw.iter().enumerate() {
            let Some(rest) = text.strip_prefix(b) else {
                continue;
            };
            if !matches!(rest.chars().next(), Some(' ' | ':' | '=') | None) {
                continue;
            }
            if text.starts_with(|c: char| c.is_whitespace()) {
                continue;
            }
            if !seen.insert((label.clone(), line_at)) {
                continue;
            }
            rows.push(Row {
                heading: None,
                text: format!("  {label}:{}  {}", line_at + 1, text.trim()),
            });
        }
    }
    (rows, known)
}

// The name's own declaration, and where a class carries it.
fn x_ray_rows(
    sources: &[Source],
    root: &Path,
    name: &str,
    items: usize,
    used: &mut Vec<usize>,
) -> Vec<Row> {
    let declares = |src: &Source, line: &Line| {
        matches!(lang::shape(src.table, line), Some("class" | "def"))
            && names(src.table, line, name)
    };
    if !sources.iter().any(|src| {
        src.table
            .pieces()
            .iter()
            .any(|(_, line)| declares(src, line))
    }) {
        return Vec::new();
    }
    let mut rows: Vec<Row> = vec![Row {
        heading: Some(format!("[x-ray] {name}")),
        text: String::new(),
    }];
    for (at, src) in sources.iter().enumerate() {
        let label = parts::rel(root, src.path);
        for (index, line) in src.table.pieces() {
            if !declares(src, line) {
                continue;
            }
            used.push(at);
            rows.push(Row {
                heading: None,
                text: format!(
                    "  {label}:{}  {}  ({}..{})",
                    index + 1,
                    exact(line),
                    index + 1,
                    src.table.end(index).unwrap_or(index) + 1
                ),
            });
            if lang::shape(src.table, line) == Some("class") {
                let mine = src.table.inner(index);
                for (member_at, _) in mine.iter().take(items) {
                    rows.push(Row {
                        heading: None,
                        text: format!("    {}", member_row(src.table, *member_at)),
                    });
                }
                if mine.len() > items {
                    rows.push(Row {
                        heading: None,
                        text: format!("    [+{} more]", mine.len() - items),
                    });
                }
            }
        }
    }
    rows
}

// Whether some class in this source is named `word`.
fn declares_class(table: &Table, word: &str) -> bool {
    table
        .pieces()
        .iter()
        .any(|(_, line)| lang::shape(table, line) == Some("class") && names(table, line, word))
}

// The module name one path stands for: the file's stem, or the package a `__init__.py` opens.
fn module_name(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    if stem != "__init__" {
        return Some(stem);
    }
    path.parent()?
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

// Whether `a` may name this file: its own name, or the last part of a dotted one.
fn answers(path: &Path, a: &str) -> bool {
    let Some(name) = module_name(path) else {
        return false;
    };
    let leaf = a.rsplit('.').next().unwrap_or(a);
    name == a || name == leaf
}

// The line a row shows: the piece's own header.
fn exact(line: &Line) -> &str {
    line.exact.as_deref().unwrap_or("")
}

// One member as a row: its span, and its own header.
fn member_row(table: &Table, at: usize) -> String {
    let end = table.end(at).unwrap_or(at);
    format!(
        "{}..{}  {}",
        at + 1,
        end + 1,
        table.line(at).map_or("", exact)
    )
}

// Lines inside one class body that bind `name` at its own level: a class variable, a folded annotation.
fn members(raw: &[&str], from: usize, to: usize, name: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (at, text) in raw.iter().enumerate().take(to.min(raw.len())).skip(from) {
        let Some(rest) = text.trim().strip_prefix(name) else {
            continue;
        };
        let next = rest.trim_start().chars().next();
        if matches!(next, Some(':' | '=' | ',')) {
            out.push(at);
        }
    }
    out
}

// Whether this piece binds `word`: the name it declares, or the class it opens.
fn names(table: &Table, line: &Line, word: &str) -> bool {
    let Some(kind) = lang::shape(table, line) else {
        return false;
    };
    if kind == "import" || kind == "from" {
        return false;
    }
    let Some(exact) = line.exact.as_deref() else {
        return false;
    };
    let rest = exact
        .strip_prefix("async def ")
        .or_else(|| exact.strip_prefix("def "))
        .or_else(|| exact.strip_prefix("class "))
        .unwrap_or(exact);
    let name: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    name == word
}

// `A.b`, exactly two identifiers: Python's own separator is the dot, and `::` belongs to the languages that write it.
fn two_segments(query: &str) -> Option<(String, String)> {
    let query = query.trim();
    let (a, b) = query.split_once('.')?;
    if b.contains('.') {
        return None;
    }
    (ident(a) && ident(b)).then(|| (a.to_string(), b.to_string()))
}

// A lone identifier.
fn single_name(query: &str) -> Option<&str> {
    let query = query.trim();
    ident(query).then_some(query)
}

fn ident(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.is_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}
