// The Go relations: `A.b` as a package's own declaration or a type's method, and a name's own declarations.

use std::collections::BTreeSet;
use std::path::Path;

use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::{self, Line, Relation, Relations, Row, Source, Table};

// How Go's relations read: a package or a type, named by a dot.
pub(crate) const RELATIONS: Relations = Relations { scan };

// Members one block lists.
const MEMBERS: usize = 12;

// The rows a query answers: `A.b` a package or type member, a lone name its declarations.
fn scan(sources: &[Source], root: &Path, query: &str) -> Relation {
    let mut used = Vec::new();
    if let Some((a, b)) = two_segments(query) {
        let mut rows = Vec::new();
        let mine = type_rows(sources, root, &a, &b, MEMBERS, &mut used);
        if mine.is_empty() {
            rows.push(Row {
                heading: None,
                text: format!("[type] {a}.{b}: no type `{a}` in this module declares `{b}`"),
            });
        } else {
            rows.push(Row {
                heading: Some(format!("[type] {a}.{b}")),
                text: String::new(),
            });
            rows.extend(mine);
        }
        rows.push(Row {
            heading: None,
            text: String::new(),
        });
        let (package, known) = package_rows(sources, root, &a, &b, &mut used);
        if package.is_empty() {
            let text = if known {
                format!("[package] {a}.{b}: package `{a}` declares no `{b}`")
            } else {
                format!("[package] {a}.{b}: no package `{a}` in this module")
            };
            rows.push(Row {
                heading: None,
                text,
            });
        } else {
            rows.push(Row {
                heading: Some(format!("[package] {a}.{b}")),
                text: String::new(),
            });
            rows.extend(package);
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

// `A.b` where `b` is declared on the type `A`: the methods Go writes outside the type, and the fields its own body holds.
// `A.b` where `b` is declared on the type `A`: the methods Go writes outside the type, and the fields its own body holds. A type and its methods need not share a file, so where `A` is found and which of its members `b` names are two separate passes over the whole module.
fn type_rows(
    sources: &[Source],
    root: &Path,
    a: &str,
    b: &str,
    items: usize,
    used: &mut Vec<usize>,
) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for (at, src) in sources.iter().enumerate() {
        let raw: Vec<&str> = src.text.lines().collect();
        let label = parts::rel(root, src.path);
        let mut mine: Vec<String> = Vec::new();
        for (index, line) in src.table.pieces() {
            match lang::shape(src.table, line) {
                Some("func") if receiver_named(line, a, b) => {
                    mine.push(member_row(src.table, index))
                }
                Some("type") if names(src.table, line, a) => {
                    for field in fields(&raw, index, b) {
                        mine.push(format!("{}  {}", field + 1, raw[field].trim()));
                    }
                }
                _ => {}
            }
        }
        if mine.is_empty() {
            continue;
        }
        used.push(at);
        rows.push(Row {
            heading: None,
            text: format!("  {label}  type {a}"),
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
    rows
}

// `A.b` where `A` is a package: the declaration `b` that package spells.
fn package_rows(
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
            if !names(src.table, item, b)
                || matches!(lang::shape(src.table, item), Some("package" | "import"))
            {
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
    }
    (rows, known)
}

// The name's own declaration, and where a type carries its methods.
fn x_ray_rows(
    sources: &[Source],
    root: &Path,
    name: &str,
    items: usize,
    used: &mut Vec<usize>,
) -> Vec<Row> {
    let declares = |table: &Table, line: &Line| {
        matches!(
            lang::shape(table, line),
            Some("func" | "type" | "var" | "const")
        ) && names(table, line, name)
    };
    if !sources.iter().any(|src| {
        src.table
            .pieces()
            .iter()
            .any(|(_, line)| declares(src.table, line))
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
            if !declares(src.table, line) {
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
            if lang::shape(src.table, line) == Some("type") {
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

// The package name one file declares on its own first line.
fn package_name(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("package") {
            let name = rest.trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

// Whether `a` may name this file's package: the name its own files declare, which is the name every other file imports it by.
fn answers(path: &Path, a: &str) -> bool {
    package_name(path).is_some_and(|name| name == a)
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

// Lines inside one block that declare `name` at its own level: a struct's field.
fn fields(raw: &[&str], at: usize, name: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (line, text) in raw.iter().enumerate().skip(at + 1) {
        let text = text.trim();
        if text.starts_with('}') {
            break;
        }
        let Some(rest) = text.strip_prefix(name) else {
            continue;
        };
        if rest.starts_with(' ') && !rest.trim().is_empty() {
            out.push(line);
        }
    }
    out
}

// Whether this piece binds `word`: the name a type, variable or constant declares, or the name or the receiver a method carries.
fn names(table: &Table, line: &Line, word: &str) -> bool {
    let Some(kind) = lang::shape(table, line) else {
        return false;
    };
    if kind == "import" || kind == "package" {
        return false;
    }
    let Some(exact) = line.exact.as_deref() else {
        return false;
    };
    // A group's own frame binds nothing: only the rows inside it do, and no group is named by one of them.
    if exact.trim() == kind || exact.trim_end().ends_with('(') {
        return false;
    }
    let Some((receiver, tail)) = declared(kind, exact) else {
        return false;
    };
    receiver.is_some_and(|held| held == word) || tail == word
}

// What one declaration binds: the receiver a method carries, and its own name. A row inside a parenthesised group carries no head word of its own, so the kind is the group's and the name is the row's first word.
fn declared<'a>(kind: &str, exact: &'a str) -> Option<(Option<&'a str>, &'a str)> {
    let rest = match exact.strip_prefix(kind) {
        Some(rest) => rest.trim_start(),
        None => exact.trim_start(),
    };
    if kind != "func" {
        return Some((None, first_word(rest)?));
    }
    match rest.strip_prefix('(') {
        Some(more) => {
            let (receiver, tail) = more.split_once(')')?;
            Some((receiver_type(receiver), first_word(tail)?))
        }
        None => Some((None, first_word(rest)?)),
    }
}

// Whether one method declares `b` on the type `a`: `func (v *Value) Add`.
fn receiver_named(line: &Line, a: &str, b: &str) -> bool {
    let Some(exact) = line.exact.as_deref() else {
        return false;
    };
    // Only a method carries a receiver, and its own text says so before anything else is read.
    let Some((receiver, tail)) = declared("func", exact) else {
        return false;
    };
    receiver == Some(a) && tail == b
}

// The type a receiver names, with a pointer and the receiver's own variable behind it: `v *Value ` reads `Value`.
fn receiver_type(receiver: &str) -> Option<&str> {
    let rest = receiver
        .split_once(' ')
        .map_or(receiver.trim(), |(_, tail)| tail.trim());
    let rest = rest.trim_start_matches('*');
    let name: &str = rest
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()?;
    (!name.is_empty()).then_some(name)
}

// `A.b`, exactly two identifiers: Go's own separator is the dot.
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

// The one identifier a head's own text begins with.
fn first_word(text: &str) -> Option<&str> {
    let word = text
        .trim_start()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()?;
    (!word.is_empty()).then_some(word)
}
