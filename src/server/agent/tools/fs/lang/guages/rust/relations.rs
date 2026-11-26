// The Rust relations: `A::b` as a module member or a type member, and a name's own declarations.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::facts::{strip_attrs, strip_vis};
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::{self, Line, Relation, Relations, Row, Source, Table};

// How Rust's relations read: a namespace or a type, indexed by `::`.
pub(crate) const RELATIONS: Relations = Relations { scan };

// Members one block lists, and impl blocks one type's x-ray lists.
const MEMBERS: usize = 12;
const IMPLS: usize = 8;

// The rows a query answers: `A::b` a module or type member, a lone name its declarations and impls.
fn scan(sources: &[Source], root: &Path, query: &str) -> Relation {
    let mut used = Vec::new();
    if let Some((a, b)) = two_segments(query) {
        let mut rows = Vec::new();
        let mine = type_rows(sources, root, &a, &b, MEMBERS, &mut used);
        if mine.is_empty() {
            rows.push(Row {
                heading: None,
                text: format!(
                    "[type] {a}::{b}: no `{a}` in this crate declares `{b}`; this is not a \
                     crate-defined type member"
                ),
            });
        } else {
            rows.push(Row {
                heading: Some(format!("[type] {a}::{b}")),
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
                format!("[module] {a}::{b}: module `{a}` declares no `{b}`")
            } else {
                format!("[module] {a}::{b}: no module `{a}` in this crate")
            };
            rows.push(Row {
                heading: None,
                text,
            });
        } else {
            rows.push(Row {
                heading: Some(format!("[module] {a}::{b}")),
                text: String::new(),
            });
            rows.extend(module);
        }
        return Relation { rows, used };
    }
    if let Some(name) = single_name(query) {
        let rows = x_ray_rows(sources, root, name, MEMBERS, IMPLS, &mut used);
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

// `A::b` where `b` is declared on the type `A`: its `impl`, `trait`, `enum` or `struct` bodies.
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
        for (index, piece) in src.table.pieces() {
            if !bears(src.table, piece) || !names(src.table, piece, a) {
                continue;
            }
            let mut mine: Vec<String> = Vec::new();
            if is_impl(src.table, piece) || lang::shape(src.table, piece) == Some("trait") {
                for (member_at, member) in src.table.inner(index) {
                    if names(src.table, member, b) && declared(src.table, member) {
                        mine.push(member_row(src.table, member_at));
                    }
                }
            }
            if matches!(lang::shape(src.table, piece), Some("enum" | "struct")) {
                let end = src.table.end(index).unwrap_or(index);
                for line in members(&raw, index + 1, end, b) {
                    mine.push(format!("{}  {}", line + 1, raw[line]));
                }
            }
            if mine.is_empty() {
                continue;
            }
            used.push(at);
            rows.push(Row {
                heading: None,
                text: format!("  {label}:{}  {}", index + 1, exact(piece)),
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

// `A::b` where `A` is a module: the item `b` that module declares.
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
        let Some(base) = src.path.parent() else {
            continue;
        };
        for (index, declaration) in src.table.pieces() {
            if lang::shape(src.table, declaration) != Some("mod")
                || !names(src.table, declaration, a)
            {
                continue;
            }
            // `mod a { … }` lives in this file; `mod a;` names a sibling file or directory.
            let end = src.table.end(index).unwrap_or(index);
            let inline = end > index;
            let mut roots: Vec<PathBuf> = Vec::new();
            if !inline {
                let dir = base.join(a);
                let single = base.join(format!("{a}.rs"));
                if dir.is_dir() {
                    roots.push(dir);
                }
                if single.is_file() {
                    roots.push(single);
                }
                if roots.is_empty() {
                    continue;
                }
            }
            known = true;
            for (other, target) in sources.iter().enumerate() {
                let inside = roots
                    .iter()
                    .any(|root| target.path == *root || target.path.starts_with(root));
                if !inside && (!inline || other != at) {
                    continue;
                }
                let label = parts::rel(root, target.path);
                for (item_at, item) in target.table.pieces() {
                    if !names(target.table, item, b)
                        || lang::shape(target.table, item) == Some("use")
                    {
                        continue;
                    }
                    if other == at && inline && !(item_at > index && item_at <= end) {
                        continue;
                    }
                    if !seen.insert((label.clone(), item_at)) {
                        continue;
                    }
                    used.push(other);
                    rows.push(Row {
                        heading: None,
                        text: format!(
                            "  {label}:{}  {}  ({}..{})",
                            item_at + 1,
                            exact(item),
                            item_at + 1,
                            target.table.end(item_at).unwrap_or(item_at) + 1
                        ),
                    });
                }
            }
        }
    }
    (rows, known)
}

// The type's own declaration, and every `impl` block for it, with their items' spans.
fn x_ray_rows(
    sources: &[Source],
    root: &Path,
    name: &str,
    items: usize,
    impls: usize,
    used: &mut Vec<usize>,
) -> Vec<Row> {
    let declares = |src: &Source, line: &Line| {
        matches!(
            lang::shape(src.table, line),
            Some("struct" | "enum" | "trait" | "union")
        ) && names(src.table, line, name)
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
        }
    }
    let mut blocks = 0usize;
    let mut listed = 0usize;
    for (at, src) in sources.iter().enumerate() {
        let label = parts::rel(root, src.path);
        for (index, line) in src.table.pieces() {
            if !is_impl(src.table, line) || !names(src.table, line, name) {
                continue;
            }
            blocks += 1;
            if listed >= impls {
                continue;
            }
            listed += 1;
            used.push(at);
            rows.push(Row {
                heading: None,
                text: format!(
                    "  {label}:{}..{}  {}",
                    index + 1,
                    src.table.end(index).unwrap_or(index) + 1,
                    exact(line)
                ),
            });
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
    if blocks == 0 {
        rows.push(Row {
            heading: None,
            text: "  [no impl blocks in this crate]".to_string(),
        });
    } else if blocks > listed {
        rows.push(Row {
            heading: None,
            text: format!("  [+{} more impl blocks]", blocks - listed),
        });
    }
    rows
}

// Whether this piece is an impl block, of either shape.
fn is_impl(table: &Table, line: &Line) -> bool {
    matches!(lang::shape(table, line), Some("impl"))
}

// Whether this piece is one of the kinds `A::b` asks about.
fn bears(table: &Table, line: &Line) -> bool {
    matches!(
        lang::shape(table, line),
        Some("impl" | "trait" | "struct" | "enum" | "union")
    )
}

// Whether this piece's body declares functions, consts, types or statics by name.
fn declared(table: &Table, line: &Line) -> bool {
    matches!(
        lang::shape(table, line),
        Some("fn" | "const" | "type" | "static")
    )
}

// Whether this piece binds `word`: its own name, or the trait and target of an `impl`.
fn names(table: &Table, line: &Line, word: &str) -> bool {
    binds(table, line, word)
}

// The line a row shows: the piece's own header, never the shape.
fn exact(line: &Line) -> &str {
    line.exact.as_deref().unwrap_or("")
}

// One member as a row: its span, and its own name.
fn member_row(table: &Table, at: usize) -> String {
    let end = table.end(at).unwrap_or(at);
    format!(
        "{}..{}  {}",
        at + 1,
        end + 1,
        table.line(at).map_or("", exact)
    )
}

// Lines inside one piece's body whose own line binds `name`: a variant, a field, a const.
fn members(raw: &[&str], from: usize, to: usize, name: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (at, text) in raw.iter().enumerate().take(to.min(raw.len())).skip(from) {
        let line = strip_vis(strip_attrs(text));
        let Some(rest) = line.strip_prefix(name) else {
            continue;
        };
        let next = rest.chars().next();
        if matches!(
            next,
            None | Some(' ' | ',' | '{' | '(' | '=' | ':' | ';' | '<')
        ) {
            out.push(at);
        }
    }
    out
}

// Whether this piece binds `word`: its own name, or the trait and target of an `impl`.
fn binds(table: &Table, line: &Line, word: &str) -> bool {
    let (Some(kind), Some(exact)) = (lang::shape(table, line), line.exact.as_deref()) else {
        return false;
    };
    match kind {
        "use" | "extern" | "macro" => false,
        "impl" => {
            let (target, trait_name) = impl_names(exact);
            target.as_deref() == Some(word) || trait_name.as_deref() == Some(word)
        }
        _ => after_word(exact, kind).and_then(bare_name).as_deref() == Some(word),
    }
}

// `impl Tr for A` → `(A, Tr)`; `impl A` → `(A, None)`. A higher-ranked bound writes `for<'a>`, with no space before its angle, so only the spaced `for` splits a header.
fn impl_names(header: &str) -> (Option<String>, Option<String>) {
    match header.rfind(" for ") {
        Some(at) => (type_name(&header[at + 5..]), own_target(&header[..at])),
        None => (own_target(header), None),
    }
}

// The text after the head word, once its modifiers are behind it.
fn after_word<'a>(header: &'a str, word: &str) -> Option<&'a str> {
    let at = header.find(word)?;
    Some(&header[at + word.len()..])
}

// `impl A` → `A`; `impl<T> A<T>` → `A`.
fn own_target(header: &str) -> Option<String> {
    type_name(after_word(header, "impl")?)
}

// The bare type a header names: references, `!`, `mut`, `dyn`, lifetimes and generics all go.
fn type_name(text: &str) -> Option<String> {
    let mut rest = text.trim();
    loop {
        let trimmed = rest.trim_start();
        if let Some(next) = trimmed.strip_prefix('&') {
            rest = next;
            continue;
        }
        if let Some(next) = trimmed.strip_prefix('!') {
            rest = next;
            continue;
        }
        if let Some(next) = trimmed.strip_prefix('<') {
            rest = skip_angle(next)?;
            continue;
        }
        if let Some(next) = trimmed.strip_prefix("mut ") {
            rest = next;
            continue;
        }
        if let Some(next) = trimmed.strip_prefix("dyn ") {
            rest = next;
            continue;
        }
        if let Some(next) = trimmed.strip_prefix("for ") {
            rest = next;
            continue;
        }
        if let Some(after) = trimmed.strip_prefix('\'') {
            let len = after
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(after.len());
            rest = &after[len..];
            continue;
        }
        rest = trimmed;
        break;
    }
    let head = rest
        .split(['<', '(', '[', '+', ' ', '\n'])
        .next()
        .unwrap_or("")
        .trim();
    let segment = head.rsplit("::").next().unwrap_or("").trim();
    let mut chars = segment.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => Some(segment.to_string()),
        _ => None,
    }
}

// Past a generic list: the text after its matching `>`.
fn skip_angle(text: &str) -> Option<&str> {
    let mut depth = 1i64;
    let chars: Vec<char> = text.chars().collect();
    for (k, c) in chars.iter().enumerate() {
        match c {
            '<' => depth += 1,
            '>' => {
                if !matches!(chars.get(k.wrapping_sub(1)), Some('-' | '=')) {
                    depth -= 1;
                }
                if depth == 0 {
                    let offset: usize = chars[..=k].iter().map(|c| c.len_utf8()).sum();
                    return Some(&text[offset..]);
                }
            }
            _ => {}
        }
    }
    None
}

// The first identifier word of a header's tail: `MyStruct<T>` → `MyStruct`, `a;` → `a`.
fn bare_name(tail: &str) -> Option<String> {
    let word: String = tail
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!word.is_empty()).then_some(word)
}

// `A::b`, exactly two identifiers.
fn two_segments(query: &str) -> Option<(String, String)> {
    let query = query.trim();
    let (a, b) = query.split_once("::")?;
    if b.contains("::") {
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
