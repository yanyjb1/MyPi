// The Go sorter: which lines are key, what each piece is, and where its block ends.

use std::path::Path;

use super::LANGUAGE;
use super::scan;
use crate::server::agent::tools::fs::lang::{Line, Table};

// The suffix this sorter answers to.
pub(crate) const SUFFIXES: &[&str] = &["go"];

// Words that open a piece: a declaration, its grouping, or the file's own clause. Go prefixes them with nothing.
const HEADS: &[&str] = &["package", "import", "func", "type", "var", "const"];

// Heads whose declaration may stand as a parenthesised group over several lines.
const GROUPED: &[&str] = &["import", "type", "var", "const"];

// A function at least this long hands out key lines inside its body.
const LONG: usize = 15;

// One file's line table.
pub(crate) fn table(_path: &Path, text: &str) -> Table {
    let raw: Vec<&str> = text.lines().collect();
    let code = scan::strip(text, token);
    let braces = scan::ends(&code, '{', '}');
    let parens = scan::frames(&code, '(', ')');
    let mut table = Table::new(raw.len(), &LANGUAGE);
    pieces(&raw, &code, &braces, &parens, &mut table);
    hats(&raw, &mut table);
    control(&code, &braces, &mut table);
    table
}

// Every declaration: the line it opens on, the line it ends on, and what it binds.
fn pieces(
    raw: &[&str],
    code: &[String],
    braces: &[Option<usize>],
    parens: &[(usize, usize)],
    table: &mut Table,
) {
    for (at, line) in code.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some(kind) = head_of(trimmed) else {
            continue;
        };
        // A group is closed by its own paren, not by a brace; a body is closed by the brace Go's own semicolon rule puts on the header's last line.
        let grouped = GROUPED.contains(&kind) && rest_of(trimmed, kind).starts_with('(');
        let (open, close) = if grouped {
            match parens.iter().find(|(open, _)| *open == at) {
                Some((_, close)) => (at, *close),
                None => (at, at),
            }
        } else {
            match span(code, at) {
                Some((open, true)) => (open, braces.get(open).copied().flatten().unwrap_or(open)),
                Some((open, false)) => (open, open),
                None => continue,
            }
        };
        let exact = body_text(raw, at, open, kind);
        let head = Some(kind.to_string());
        if !grouped {
            table.span(at, close, head, Some(exact));
            continue;
        }
        // A group binds no name on its own line: the rows inside it do, and the group is only the frame around them. A row that opens a body of its own keys that body's lines itself, so its interior is left alone. An import group stays one piece: what it names is a path, not a declaration.
        table.span(at, close, head, Some(exact));
        if kind == "import" {
            continue;
        }
        let mut body = at;
        for (row, text) in raw.iter().enumerate().take(close + 1).skip(at + 1) {
            if row <= body {
                continue;
            }
            if parens
                .iter()
                .any(|(opens, closes)| *opens == row && *closes > row)
            {
                continue;
            }
            match row_head(text.trim()) {
                Some(named) => {
                    // A row that opens a body reaches to that body's own close, the way a top-level declaration does: `Group struct {` covers its fields.
                    let end = match braces.get(row).copied().flatten() {
                        Some(end) if end > row => end,
                        _ => row,
                    };
                    table.reach(row, end, Some(kind.to_string()), Some(named));
                    body = end;
                }
                None => table.reach(row, row, None, None),
            }
        }
    }
}

// Comment blocks: their first and last lines are key, whatever they hold.
fn hats(raw: &[&str], table: &mut Table) {
    let n = raw.len();
    let mut at = 0;
    while at < n {
        if !is_hat(raw[at].trim_start()) {
            at += 1;
            continue;
        }
        let start = at;
        let mut end = at;
        let mut block = raw[at].trim_start().starts_with("/*") && !raw[at].contains("*/");
        let mut probe = at + 1;
        while probe < n {
            let line = raw[probe].trim_start();
            if block {
                end = probe;
                if line.contains("*/") {
                    block = false;
                }
                probe += 1;
                continue;
            }
            if !is_hat(line) {
                break;
            }
            end = probe;
            block = line.starts_with("/*") && !line.contains("*/");
            probe += 1;
        }
        // A hat that owns nothing is prose, and prose gets no number — except at the file's own head, which a change can name.
        if start == 0 || (end + 1 < n && !raw[end + 1].trim().is_empty()) {
            table.span(start, end, None, None);
        }
        at = end + 1;
    }
}

// A long function's own control-flow blocks: the branches, loops and switches of its body.
fn control(code: &[String], braces: &[Option<usize>], table: &mut Table) {
    let long: Vec<(usize, usize)> = table
        .pieces()
        .into_iter()
        .filter(|(_, line)| shape(line) == Some("func"))
        .filter_map(|(at, _)| table.end(at).map(|end| (at, end)))
        .collect();
    for (head, close) in long {
        if close <= head + LONG {
            continue;
        }
        for (at, line) in code.iter().enumerate().take(close).skip(head + 1) {
            if table.key(at) || !control_head(line.trim()) {
                continue;
            }
            let end = braces.get(at).copied().flatten().unwrap_or(at);
            table.span(at, end, None, None);
        }
    }
}

// The head word of a declaration; `None` when the line opens no piece.
fn head_of(line: &str) -> Option<&'static str> {
    let word = line
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>();
    HEADS.iter().copied().find(|head| *head == word)
}

// What stands after the head word on its own line.
fn rest_of<'a>(line: &'a str, kind: &str) -> &'a str {
    line.strip_prefix(kind).unwrap_or(line).trim_start()
}

// A header's ends: the line the body opens on, and whether there is a body at all. Go puts the brace on the line that closes the signature, so the first brace outside a paren or a bracket is the body's.
fn span(code: &[String], start: usize) -> Option<(usize, bool)> {
    let mut paren = 0i64;
    let mut square = 0i64;
    for (at, line) in code.iter().enumerate().skip(start) {
        for ch in line.chars() {
            match ch {
                '(' => paren += 1,
                ')' => paren -= 1,
                '[' => square += 1,
                ']' => square -= 1,
                '{' if paren <= 0 && square <= 0 => return Some((at, true)),
                _ => {}
            }
        }
        if paren <= 0 && square <= 0 {
            return Some((start, false));
        }
    }
    None
}

// A declaration's own header, with the body's opener behind it: `func Foo() int {` → `func Foo() int`.
fn body_text(raw: &[&str], head: usize, open: usize, kind: &str) -> String {
    let mut text = String::new();
    for line in raw.iter().take(open + 1).skip(head) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(line);
    }
    let text = text.trim();
    if GROUPED.contains(&kind) && rest_of(text, kind).starts_with('(') {
        return text.trim_end_matches('(').trim_end_matches(' ').to_string();
    }
    for tail in ["{}", "{"] {
        if let Some(rest) = text.strip_suffix(tail) {
            return rest.trim_end().to_string();
        }
    }
    text.to_string()
}

// The kind of piece a line opens: the head word of its label.
pub(crate) fn shape(line: &Line) -> Option<&str> {
    let label = line.label.as_deref()?;
    label.split_whitespace().find(|word| HEADS.contains(word))
}

// One row inside a parenthesised group, as what it binds: `MaxSize = 10` keeps its own text, and `Group struct {` loses the brace its body opens with.
fn row_head(text: &str) -> Option<String> {
    let name: String = text
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }
    Some(text.trim_end_matches('{').trim_end().to_string())
}

fn is_hat(line: &str) -> bool {
    line.starts_with("//") || line.starts_with("/*")
}

// A line that opens a block inside a long function, or the second half of `} else {`.
fn control_head(line: &str) -> bool {
    let head = line.strip_prefix('}').map_or(line, str::trim_start);
    for word in ["if", "for", "switch", "select", "else"] {
        if let Some(rest) = head.strip_prefix(word)
            && (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('{'))
        {
            return true;
        }
    }
    // A closure the line opens: `go func() {` and `defer func() {`.
    line.ends_with('{') && line.contains("func(")
}

// Go's delimiters: a line comment, a block comment, an interpreted literal, a raw literal and a rune.
pub(super) fn token(blank: &mut scan::Blank, c: char) -> bool {
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
        '"' => {
            blank.skip(1);
            blank.gap();
            blank.skip_quote('"');
        }
        '`' => {
            blank.skip(1);
            blank.gap();
            blank.skip_raw();
        }
        '\'' => {
            blank.skip(1);
            blank.gap();
            blank.skip_quote('\'');
        }
        _ => return false,
    }
    true
}
