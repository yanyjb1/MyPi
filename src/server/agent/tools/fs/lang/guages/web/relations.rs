// The web relations: one DOM anchor read from a page, a stylesheet and a script at once, plus a script's own members.

use std::path::Path;

use super::{css, html, js};
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::{Line, Relation, Relations, Row, Source};

// How the web's relations read: `#id` and `.class` across all three kinds, `Class.method` and a bare name inside scripts.
pub(crate) const RELATIONS: Relations = Relations { scan };

// The rows a query answers, and the sources they drew from.
fn scan(sources: &[Source], root: &Path, query: &str) -> Relation {
    let query = query.trim();
    let mut used: Vec<usize> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let anchor = query
        .chars()
        .next()
        .filter(|c| *c == '#' || *c == '.')
        .map(|kind| (kind, query[1..].trim().to_string()));
    if let Some((kind, name)) = anchor {
        if !name.is_empty() {
            dom(sources, root, kind, &name, &mut rows, &mut used);
        }
        return Relation { rows, used };
    }
    if let Some((a, b)) = two_names(query) {
        members(sources, root, &a, &b, &mut rows, &mut used);
        return Relation { rows, used };
    }
    if let Some(name) = lone(query) {
        declared(sources, root, name, &mut rows, &mut used);
    }
    Relation { rows, used }
}

// The three ends of one DOM anchor: where a page defines it, where a stylesheet styles it, where a script reaches for it.
fn dom(
    sources: &[Source],
    root: &Path,
    kind: char,
    name: &str,
    rows: &mut Vec<Row>,
    used: &mut Vec<usize>,
) {
    rows.push(Row {
        heading: Some(format!("[{}] {kind}{name}", kind_word(kind))),
        text: String::new(),
    });
    let mut hits: Vec<Row> = Vec::new();
    for (at, src) in sources.iter().enumerate() {
        let label = parts::rel(root, src.path);
        let mut mine: Vec<Row> = Vec::new();
        match super::kind(src.path) {
            Some("html") => {
                let tags = html::scan(src.text);
                for (own, held, line) in html::anchors(&tags) {
                    if own == kind && held == name {
                        mine.push(Row {
                            heading: None,
                            text: format!("  {label}:{}  <{}>", line + 1, tag_of(&tags, line)),
                        });
                    }
                }
            }
            Some("css") => {
                let table = css::table(src.text);
                for (own, held, line) in css::anchors(&table) {
                    if own != kind || held != name {
                        continue;
                    }
                    let exact = table.line(line).and_then(|one| one.exact.clone());
                    mine.push(Row {
                        heading: None,
                        text: format!(
                            "  {label}:{}  {}  ({}..{})",
                            line + 1,
                            exact.unwrap_or_default(),
                            line + 1,
                            table.end(line).unwrap_or(line) + 1
                        ),
                    });
                }
            }
            Some("js") => {
                let scanned = js::strip(src.text);
                for (line, said) in calls(&scanned, src.text, kind, name) {
                    mine.push(Row {
                        heading: None,
                        text: format!("  {label}:{}  {said}", line + 1),
                    });
                }
            }
            _ => {}
        }
        if !mine.is_empty() {
            used.push(at);
            hits.append(&mut mine);
        }
    }
    if hits.is_empty() {
        rows.push(Row {
            heading: None,
            text: format!("  no page, stylesheet or script in scope names {kind}{name}"),
        });
        return;
    }
    rows.append(&mut hits);
}

// What the page's tag on one line is called.
fn tag_of(tags: &[html::Tag], line: usize) -> String {
    tags.iter()
        .find(|tag| tag.line == line && !tag.close)
        .map(|tag| tag.name.clone())
        .unwrap_or_default()
}

// The word a heading reads by.
fn kind_word(kind: char) -> &'static str {
    if kind == '#' { "id" } else { "class" }
}

// The lines of one script that reach for this anchor: `getElementById`, `querySelector` and the class list.
fn calls(scanned: &js::Scanned, text: &str, kind: char, name: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (at, raw) in text.lines().enumerate() {
        if !scanned.live.get(at).copied().unwrap_or(false) {
            continue;
        }
        let line = raw.trim();
        if reaches(line, kind, name) || (kind == '.' && mentions_class(line, name)) {
            out.push((at, line.to_string()));
        }
    }
    out
}

// Whether a line reaches this anchor wherever a script may write it: `getElementById('x')` takes the bare name, and every other form writes it as a quoted selector or fragment — compound, descendant, or a call.
fn reaches(line: &str, kind: char, name: &str) -> bool {
    let bare = kind == '#' && line.contains("getElementById(");
    literals(line)
        .into_iter()
        .any(|held| (bare && held == name) || carries(held, kind, name))
}

// Every text one line wraps in quotes.
fn literals(line: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut open: Option<(char, usize)> = None;
    for (at, c) in line.char_indices() {
        match open {
            None if c == '"' || c == '\'' => open = Some((c, at + c.len_utf8())),
            Some((mark, from)) if c == mark => {
                out.push(&line[from..at]);
                open = None;
            }
            _ => {}
        }
    }
    out
}

// Whether one quoted selector names this anchor, wherever the anchor stands inside it: `'.card'`, `'div.card > .title'` and `'#list .card'` all name `.card`. A name that goes on letters is another name: `.cardio` is not `.card`, and `#card-list` is not `#card`.
fn carries(selector: &str, kind: char, name: &str) -> bool {
    let want = format!("{kind}{name}");
    let mut rest = selector;
    while let Some(at) = rest.find(&want) {
        let after = &rest[at + want.len()..];
        if !after.chars().next().is_some_and(name_char) {
            return true;
        }
        rest = after;
    }
    false
}

// Whether this character may go on a name.
fn name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

// Whether a line names this class through the class list.
fn mentions_class(line: &str, name: &str) -> bool {
    for call in [
        "classList.add(",
        "classList.remove(",
        "classList.toggle(",
        "classList.contains(",
    ] {
        let Some(at) = line.find(call) else {
            continue;
        };
        let tail = &line[at + call.len()..];
        let quoted = [format!("\"{name}\""), format!("'{name}'")];
        if quoted.iter().any(|held| tail.contains(held.as_str())) {
            return true;
        }
    }
    false
}

// The methods one class of the scope declares: `Class.method`.
fn members(
    sources: &[Source],
    root: &Path,
    a: &str,
    b: &str,
    rows: &mut Vec<Row>,
    used: &mut Vec<usize>,
) {
    rows.push(Row {
        heading: Some(format!("[method] {a}.{b}")),
        text: String::new(),
    });
    let mut found = false;
    for (at, src) in sources.iter().enumerate() {
        if super::kind(src.path) != Some("js") {
            continue;
        }
        let table = super::js::table(src.text);
        let label = parts::rel(root, src.path);
        for (class_at, line) in table.pieces() {
            if line.label.as_deref() != Some("class") || !names(line, a) {
                continue;
            }
            for (method_at, method) in table.inner(class_at) {
                if method.label.as_deref() != Some("method") || !names(method, b) {
                    continue;
                }
                found = true;
                if !used.contains(&at) {
                    used.push(at);
                }
                rows.push(Row {
                    heading: None,
                    text: format!(
                        "  {label}:{}..{}  {}",
                        method_at + 1,
                        table.end(method_at).unwrap_or(method_at) + 1,
                        method.exact.clone().unwrap_or_default()
                    ),
                });
            }
        }
    }
    if !found {
        rows.push(Row {
            heading: None,
            text: format!("  no class `{a}` in scope declares `{b}`"),
        });
    }
}

// A lone name's declarations inside scripts.
fn declared(
    sources: &[Source],
    root: &Path,
    name: &str,
    rows: &mut Vec<Row>,
    used: &mut Vec<usize>,
) {
    for (at, src) in sources.iter().enumerate() {
        if super::kind(src.path) != Some("js") {
            continue;
        }
        let table = super::js::table(src.text);
        let label = parts::rel(root, src.path);
        for (line_at, line) in table.pieces() {
            if !names(line, name) {
                continue;
            }
            if !used.contains(&at) {
                used.push(at);
            }
            rows.push(Row {
                heading: None,
                text: format!(
                    "  {label}:{}..{}  {}",
                    line_at + 1,
                    table.end(line_at).unwrap_or(line_at) + 1,
                    line.exact.clone().unwrap_or_default()
                ),
            });
        }
    }
}

// Whether this piece binds `word`.
fn names(line: &Line, word: &str) -> bool {
    let Some(exact) = line.exact.as_deref() else {
        return false;
    };
    let tail = match line.label.as_deref() {
        Some("var") => exact.split(['=', ' ']).next().unwrap_or(exact),
        _ => exact
            .rsplit_once(' ')
            .map(|(_, tail)| tail)
            .unwrap_or(exact),
    };
    tail.trim() == word
}

// `Class.method`, exactly two names.
fn two_names(query: &str) -> Option<(String, String)> {
    let (a, b) = query.split_once('.')?;
    if b.contains('.') || a.starts_with('.') {
        return None;
    }
    (ident(a) && ident(b)).then(|| (a.to_string(), b.to_string()))
}

// A lone identifier.
fn lone(query: &str) -> Option<&str> {
    ident(query).then_some(query)
}

fn ident(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}
