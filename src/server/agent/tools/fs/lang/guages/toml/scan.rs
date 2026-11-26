// The TOML scan: which line each table header opens on, and how far it reaches, read off the headers and key lines.

// One table of a document: the line its header stands on, the line its last entry ends on, and the dotted name it carries.
pub(crate) struct Shape {
    // 1-based.
    pub(crate) open: usize,

    // 1-based; equal to `open` for a header no entry follows.
    pub(crate) close: usize,

    // The name without its brackets; `None` for the document's own leading keys, which no header introduces.
    pub(crate) name: Option<String>,
}

// The delimiters that open a string running over several lines.
const BASIC: &str = "\"\"\"";
const LITERAL: &str = "'''";

// Every table of one document, in line order: the leading keys as one piece, then one piece per header. A key line belongs to the table above it; a header closes the table above it and opens its own.
pub(crate) fn shapes(text: &str) -> Vec<Shape> {
    let mut out: Vec<Shape> = Vec::new();
    let mut current: Option<Shape> = None;
    let mut leading: Option<(usize, usize)> = None;
    let mut string: Option<&'static str> = None;
    for (at, line) in text.lines().enumerate() {
        // A line of a multi-line string is that string's text: no header and no entry stands there.
        let quoted = string.is_some();
        string = after(line, string);
        if quoted {
            continue;
        }
        if let Some(name) = header(line) {
            if let Some(held) = current.take() {
                out.push(held);
            }
            current = Some(Shape {
                open: at + 1,
                close: at + 1,
                name: Some(name),
            });
            continue;
        }
        if !entry(line) {
            continue;
        }
        if current.is_none() {
            match &mut leading {
                Some(held) => held.1 = at + 1,
                None => leading = Some((at + 1, at + 1)),
            }
        }
        if let Some(held) = current.as_mut() {
            held.close = at + 1;
        }
    }
    // A table ends on its last entry, never on the blank lines below it.
    if let Some(held) = current.take() {
        out.push(held);
    }
    if let Some((open, close)) = leading {
        out.insert(
            0,
            Shape {
                open,
                close,
                name: None,
            },
        );
    }
    out
}

// The multi-line string still open after this line, if any. A comment holds no delimiter, so it leaves the state alone.
fn after(line: &str, open: Option<&'static str>) -> Option<&'static str> {
    let mut open = open;
    let mut rest = line;
    if open.is_none() && line.trim_start().starts_with('#') {
        return None;
    }
    loop {
        let Some(token) = open else {
            let found = [BASIC, LITERAL]
                .into_iter()
                .filter_map(|token| rest.find(token).map(|at| (at, token)))
                .min_by_key(|(at, _)| *at);
            let (at, token) = found?;
            open = Some(token);
            rest = &rest[at + token.len()..];
            continue;
        };
        let Some(at) = rest.find(token) else {
            return Some(token);
        };
        // A backslash keeps a basic string's delimiter from closing it.
        if token == BASIC && escaped(&rest[..at]) {
            rest = &rest[at + token.len()..];
            continue;
        }
        open = None;
        rest = &rest[at + token.len()..];
    }
}

// Whether the text ends in an odd run of backslashes, which escapes the character after it.
fn escaped(text: &str) -> bool {
    text.chars().rev().take_while(|one| *one == '\\').count() % 2 == 1
}

// The name a header line carries: `[a.b]` reads `a.b`, `[[a]]` reads `a`. A line whose brackets hold something no key may be — `[1, 2]`, a value's own array — carries no name.
fn header(line: &str) -> Option<String> {
    let line = line.trim();
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    let inner = inner
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(inner);
    let name = inner.trim();
    (!name.is_empty() && named(name)).then(|| name.to_string())
}

// Whether every dot-separated segment of a name is a bare key or a quoted one.
fn named(name: &str) -> bool {
    name.split('.').all(|part| {
        let part = part.trim();
        let quoted = part.len() > 1
            && (part.starts_with('"') && part.ends_with('"')
                || part.starts_with('\'') && part.ends_with('\''));
        quoted
            || !part.is_empty()
                && part
                    .chars()
                    .all(|one| one.is_ascii_alphanumeric() || matches!(one, '_' | '-'))
    })
}

// Whether this line opens a table's entry: a bare key, or a dotted one, then `=`. A blank line, a comment and a line inside a multi-line value carry nothing; a value's own continuation is not a key because it holds no `=` before its first quote.
pub(crate) fn entry(line: &str) -> bool {
    let line = line.trim();
    let Some((key, _)) = line.split_once('=') else {
        return false;
    };
    let key = key.trim();
    if key.is_empty() || key.starts_with('#') {
        return false;
    }
    let bare = key.trim_matches('"').trim_matches('\'');
    !bare.is_empty()
        && bare
            .chars()
            .all(|one| one.is_ascii_alphanumeric() || matches!(one, '_' | '-' | '.' | ' '))
}
