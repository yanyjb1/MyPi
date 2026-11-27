// The CP relations: `A::b` as a type's member or a namespace's declaration, and a lone name's own declarations.

use std::collections::BTreeSet;
use std::path::Path;

use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::{self, Line, Relation, Relations, Row, Source, Table};

// How CP's relations read: C++'s `::`, the only separator both halves of the language share.
pub(crate) const RELATIONS: Relations = Relations { scan };

// Members one block lists, and the declarations one name's x-ray lists.
const MEMBERS: usize = 12;
const DECLARES: usize = 8;

// The kinds whose body is a scope.
const SCOPES: &[&str] = &["class", "struct", "union", "namespace"];

// The rows a query answers: `A::b` a member or a scope's declaration, a lone name its declarations.
fn scan(sources: &[Source], root: &Path, query: &str) -> Relation {
    let mut used = Vec::new();
    if let Some((a, b)) = two_segments(query) {
        let mut rows = Vec::new();
        let mut mine = type_rows(sources, root, &a, &b, &mut used);
        mine.extend(out_of_line(sources, root, &a, &b, &mut used));
        if mine.is_empty() {
            rows.push(Row {
                heading: None,
                text: format!("[type] {a}::{b}: no `{a}` in this project declares `{b}`"),
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
        let (scope, known) = scope_rows(sources, root, &a, &b, &mut used);
        if scope.is_empty() {
            let text = if known {
                format!("[scope] {a}::{b}: namespace `{a}` declares no `{b}`")
            } else {
                format!("[scope] {a}::{b}: no namespace `{a}` in this project")
            };
            rows.push(Row {
                heading: None,
                text,
            });
        } else {
            rows.push(Row {
                heading: Some(format!("[scope] {a}::{b}")),
                text: String::new(),
            });
            rows.extend(scope);
        }
        return Relation { rows, used };
    }
    if let Some(name) = single_name(query) {
        let rows = x_ray_rows(sources, root, name, &mut used);
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

// `A::b` where `b` is declared inside the type `A`: its methods, its data members, and an enum's enumerators.
fn type_rows(sources: &[Source], root: &Path, a: &str, b: &str, used: &mut Vec<usize>) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for (at, src) in sources.iter().enumerate() {
        let raw: Vec<&str> = src.text.lines().collect();
        let label = parts::rel(root, src.path);
        for (index, piece) in src.table.pieces() {
            let kind = lang::shape(src.table, piece);
            if !SCOPES.contains(&kind.unwrap_or_default()) || named(piece) != Some(a) {
                continue;
            }
            let mut mine: Vec<String> = Vec::new();
            for (member_at, member) in src.table.inner(index) {
                if named(member) == Some(b) {
                    mine.push(member_row(src.table, member_at));
                }
            }
            // A member defined outside its type: `double Circle::area() const` declares `Circle`'s `area` wherever it stands.
            for (other_at, other) in src.table.pieces() {
                if other_at != index && qualified(other, a, b) {
                    mine.push(member_row(src.table, other_at));
                }
            }
            if matches!(kind, Some("struct" | "union" | "class")) {
                let end = src.table.end(index).unwrap_or(index);
                for line in bound(&raw, index + 1, end, b) {
                    let text = raw[line].trim();
                    if !mine.iter().any(|held| held.contains(text)) {
                        mine.push(format!("{}  {}", line + 1, text));
                    }
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
            for row in mine.iter().take(MEMBERS) {
                rows.push(Row {
                    heading: None,
                    text: format!("    {row}"),
                });
            }
            if mine.len() > MEMBERS {
                rows.push(Row {
                    heading: None,
                    text: format!("    [+{} more]", mine.len() - MEMBERS),
                });
            }
        }
    }
    rows
}

// The out-of-line definitions of one type's members: `double Circle::area() const` declares `Circle`'s `area` from whatever file holds it, and a file that never opens the type still declares it.
fn out_of_line(
    sources: &[Source],
    root: &Path,
    a: &str,
    b: &str,
    used: &mut Vec<usize>,
) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for (at, src) in sources.iter().enumerate() {
        let label = parts::rel(root, src.path);
        for (index, piece) in src.table.pieces() {
            if !qualified(piece, a, b) {
                continue;
            }
            used.push(at);
            rows.push(Row {
                heading: None,
                text: format!(
                    "  {label}:{}  {}  ({}..{})",
                    index + 1,
                    exact(piece),
                    index + 1,
                    src.table.end(index).unwrap_or(index) + 1
                ),
            });
        }
    }
    rows
}

// `A::b` where `A` is a namespace: the declaration `b` that namespace holds.
fn scope_rows(
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
        if declares_scope(src.table, a) {
            known = true;
        }
        let label = parts::rel(root, src.path);
        for (index, piece) in src.table.pieces() {
            if named(piece) != Some(b) || !in_scope(src.table, index, a) {
                continue;
            }
            if !seen.insert((label.clone(), index)) {
                continue;
            }
            used.push(at);
            rows.push(Row {
                heading: None,
                text: format!(
                    "  {label}:{}  {}  ({}..{})",
                    index + 1,
                    exact(piece),
                    index + 1,
                    src.table.end(index).unwrap_or(index) + 1
                ),
            });
        }
    }
    (rows, known)
}

// The name's own declarations, wherever they stand, and every macro that defines it.
fn x_ray_rows(sources: &[Source], root: &Path, name: &str, used: &mut Vec<usize>) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for (at, src) in sources.iter().enumerate() {
        let label = parts::rel(root, src.path);
        let mut mine: Vec<String> = Vec::new();
        for (index, piece) in src.table.pieces() {
            if named(piece) != Some(name) {
                continue;
            }
            let chain: Vec<String> = src
                .table
                .around(index)
                .into_iter()
                .filter(|(outer, _)| *outer != index)
                .filter_map(|(_, outer)| named(outer).map(str::to_string))
                .collect();
            let place = if chain.is_empty() {
                String::new()
            } else {
                format!("  in {}", chain.join(" > "))
            };
            mine.push(format!(
                "{}..{}  {}{place}",
                index + 1,
                src.table.end(index).unwrap_or(index) + 1,
                exact(piece)
            ));
        }
        for (line, text) in macros(src.text, name) {
            mine.push(format!("{}  {text}", line + 1));
        }
        if mine.is_empty() {
            continue;
        }
        used.push(at);
        rows.push(Row {
            heading: None,
            text: format!("  {label}"),
        });
        for row in mine.iter().take(DECLARES) {
            rows.push(Row {
                heading: None,
                text: format!("    {row}"),
            });
        }
        if mine.len() > DECLARES {
            rows.push(Row {
                heading: None,
                text: format!("    [+{} more]", mine.len() - DECLARES),
            });
        }
    }
    if rows.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Row> = vec![Row {
        heading: Some(format!("[x-ray] {name}")),
        text: String::new(),
    }];
    out.extend(rows);
    out
}

// Whether a file opens a scope named `a`.
fn declares_scope(table: &Table, a: &str) -> bool {
    table.pieces().iter().any(|(_, line)| {
        SCOPES.contains(&lang::shape(table, line).unwrap_or_default()) && named(line) == Some(a)
    })
}

// Whether the piece at `index` stands inside a namespace named `a`.
fn in_scope(table: &Table, index: usize, a: &str) -> bool {
    table
        .around(index)
        .iter()
        .any(|(_, outer)| lang::shape(table, outer) == Some("namespace") && named(outer) == Some(a))
}

// The name a declaration binds: the type, the namespace, the function, or the alias.
fn named(line: &Line) -> Option<&str> {
    let text = line.exact.as_deref()?.trim();
    let kind = line.label.as_deref()?;
    match kind {
        "namespace" | "class" | "struct" | "union" | "enum" => {
            let mut words = text.split_whitespace();
            while let Some(word) = words.next() {
                if raw_word(word) != kind {
                    continue;
                }
                let mut next = words.next()?;
                // `enum class Color` names the enum after the class key.
                if kind == "enum" && matches!(raw_word(next).as_str(), "class" | "struct") {
                    next = words.next()?;
                }
                return Some(first_identifier(next));
            }
            None
        }
        // `using namespace` binds no name, `using A::B` binds `B`, and `using X = Y` binds `X`.
        "using" => {
            let rest = text.strip_prefix("using")?.trim_start();
            if rest.starts_with("namespace") {
                return None;
            }
            if let Some((alias, _)) = rest.split_once('=') {
                return Some(first_identifier(alias.trim()));
            }
            Some(first_identifier(rest.rsplit("::").next().unwrap_or(rest)))
        }
        "typedef" => text.split_whitespace().next_back().map(first_identifier),
        _ => {
            if text.contains("operator") {
                return Some("operator");
            }
            let before = &text[..text.find('(')?];
            let tail = before
                .trim_end()
                .rsplit(|c: char| c.is_whitespace() || matches!(c, '*' | '&' | ':'))
                .find(|word| !word.is_empty())?;
            Some(tail.trim_start_matches('~'))
        }
    }
}

// Lines inside one scope's body that bind `name` at its own depth: a data member, an enumerator.
fn bound(raw: &[&str], from: usize, to: usize, name: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (at, text) in raw.iter().enumerate().take(to.min(raw.len())).skip(from) {
        if binds(text.trim(), name) {
            out.push(at);
        }
    }
    out
}

// Whether one line binds `name` without opening a piece: a data member, an enumerator, an alias. A call is no binding.
fn binds(text: &str, name: &str) -> bool {
    if text.is_empty() || text.contains('(') || text.starts_with(['/', '#']) {
        return false;
    }
    let cut = text
        .split([';', '=', ',', '['])
        .next()
        .unwrap_or(text)
        .trim_end();
    let tail = cut
        .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
        .find(|word| !word.is_empty());
    tail == Some(name)
}

// Whether a declaration names a member of type `a`: `double Circle::area() const`, `void ns::Circle::area()` and `void Circle<T>::area()` are all `Circle`'s own.
fn qualified(line: &Line, a: &str, b: &str) -> bool {
    let text = line.exact.as_deref().unwrap_or("");
    let head = text.split('(').next().unwrap_or(text);
    let parts: Vec<&str> = head.split("::").collect();
    let Some((member, above)) = parts.split_last() else {
        return false;
    };
    if parts.len() < 2 || first_identifier(member.trim_start()) != b {
        return false;
    }
    // A scope may carry a template's own parameters, and the type's name stands before them.
    let scope = above
        .last()
        .map_or("", |one| one.rsplit([' ', '*', '&']).next().unwrap_or(one))
        .split('<')
        .next()
        .unwrap_or("");
    scope == a
}

// Every `#define` of `name`, with its line.
fn macros(text: &str, name: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (at, line) in text.lines().enumerate() {
        let rest = line.trim_start();
        let Some(rest) = rest.strip_prefix('#') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix("define") else {
            continue;
        };
        let rest = rest.trim_start();
        if rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<String>()
            == name
        {
            out.push((at, line.trim().to_string()));
        }
    }
    out
}

// A token's leading identifier.
fn first_identifier(token: &str) -> &str {
    let end = token
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(token.len());
    &token[..end]
}

fn raw_word(token: &str) -> String {
    first_identifier(token).to_string()
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
