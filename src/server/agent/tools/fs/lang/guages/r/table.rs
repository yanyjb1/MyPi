// The R sorter: which lines are key, what each piece is, and where its block ends.

use std::path::Path;

use super::LANGUAGE;
use super::scan;
use crate::server::agent::tools::fs::lang::{Line, Table};

// The suffixes this sorter answers to. R's own convention is the upper-case `.R`, and lower case is written too.
pub(crate) const SUFFIXES: &[&str] = &["R", "r"];

// A definition whose value is written on its own line: the words a binding may put on the right.
const MAKERS: &[&str] = &["function", "\\(", "R6Class", "setRefClass"];

// A definition written as a bare call, its name inside the call instead of left of an assignment.
const CALLS: &[&str] = &["setClass", "setGeneric", "setMethod", "setRefClass"];

// A block that stands at the file's own level without binding anything.
const BLOCKS: &[&str] = &["if", "for", "while", "repeat"];

// One file's line table. Only the file's own level is a piece: a definition inside a call or a body belongs to whatever holds it.
pub(crate) fn table(_path: &Path, text: &str) -> Table {
    let text = scan::body(text);
    let raw: Vec<&str> = text.lines().collect();
    let code = scan::strip(text);
    let braces = scan::ends(&code, '{', '}');
    let parens = scan::ends(&code, '(', ')');
    let depth = depths(&code);
    let mut table = Table::new(raw.len(), &LANGUAGE);
    for (at, line) in code.iter().enumerate() {
        let trimmed = line.trim_start();
        if depth[at] != 0 || trimmed.is_empty() || trimmed.starts_with('}') {
            continue;
        }
        let Some((kind, name)) = kind_of(trimmed) else {
            continue;
        };
        let close = span(&code, &braces, &parens, at);
        // An `if` handed over to an `else` is one piece: the branch after it belongs to the same block.
        let close = if kind == "if" {
            else_tail(&code, &braces, close)
        } else {
            close
        };
        let exact = header_text(&code, &raw, at, close, kind);
        let label = match name {
            Some(name) => format!("{name} {kind}"),
            None => kind.to_string(),
        };
        table.span(at, close, Some(label), Some(exact));
    }
    table
}

// The kind of piece a line opens, in R's own words.
pub(crate) fn shape(line: &Line) -> Option<&str> {
    let label = line.label.as_deref()?;
    let word = label.split_whitespace().last()?;
    match word {
        "function" | r"\(" => Some("function"),
        "R6Class" | "setRefClass" | "setClass" | "setGeneric" | "setMethod" => Some("class"),
        other => BLOCKS.iter().copied().find(|head| *head == other),
    }
}

// The name a piece binds, read back from the label `shape` was handed.
pub(super) fn named(line: &Line) -> Option<&str> {
    let label = line.label.as_deref()?;
    let name = label.rsplit_once(' ')?.0;
    (!name.is_empty()).then_some(name)
}

// The brace and paren depth each line starts at, so a piece is taken only where the file's own level is.
fn depths(code: &[String]) -> Vec<i64> {
    let mut out = Vec::with_capacity(code.len());
    let mut brace = 0i64;
    let mut paren = 0i64;
    for line in code {
        out.push(brace + paren);
        for ch in line.chars() {
            match ch {
                '{' => brace += 1,
                '}' => brace = (brace - 1).max(0),
                '(' => paren += 1,
                ')' => paren = (paren - 1).max(0),
                _ => {}
            }
        }
    }
    out
}

// What a line opens, and what it binds: a definition on the right of an assignment, a call that defines in its own name, or a block that binds nothing.
fn kind_of(line: &str) -> Option<(&'static str, Option<String>)> {
    for head in CALLS {
        let Some(rest) = line.strip_prefix(*head) else {
            continue;
        };
        if rest.trim_start().starts_with('(') {
            return Some((head, None));
        }
    }
    for head in BLOCKS {
        let Some(rest) = line.strip_prefix(*head) else {
            continue;
        };
        let opens = rest.starts_with(' ') || rest.starts_with('(');
        if opens && (*head != "repeat" || rest.trim_start().starts_with('{')) {
            return Some((head, None));
        }
    }
    let (left, right) = assignment(line)?;
    let right = right.trim_start();
    let kind = MAKERS.iter().copied().find(|maker| {
        right.starts_with(maker)
            && (maker.ends_with('(') || right[maker.len()..].trim_start().starts_with('('))
    })?;
    let name = binding(left)?;
    Some((kind, Some(name)))
}

// The two sides of an assignment: `a <- b`, `a = b`, with the assignment's own arrow left off the right.
fn assignment(line: &str) -> Option<(&str, &str)> {
    let at = line.find("<-").or_else(|| line.find('='))?;
    let right = line[at..].trim_start_matches(['<', '=', '-']);
    Some((&line[..at], right))
}

// The name an assignment binds, when the left side is one name.
fn binding(left: &str) -> Option<String> {
    let name = left.trim();
    // A backtick name may hold anything but a backtick: `odd name` binds one name like any other.
    if let Some(inner) = name
        .strip_prefix('`')
        .and_then(|rest| rest.strip_suffix('`'))
    {
        return (!inner.is_empty() && !inner.contains('`')).then(|| inner.to_string());
    }
    let ok = !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '.' || c == '_')
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.'));
    ok.then(|| name.to_string())
}

// The line a piece ends on: the widest body or call it opens, or the line it stands whole on.
fn span(code: &[String], braces: &[Option<usize>], parens: &[Option<usize>], at: usize) -> usize {
    let tail = parens.get(at).copied().flatten().unwrap_or(at);
    let mut end = tail;
    for line in at..=tail {
        if let Some(close) = braces.get(line).copied().flatten() {
            end = end.max(close);
        }
    }
    if end > tail {
        return end;
    }
    // A body written on the line below the header still belongs to this piece, blank lines aside.
    let mut line = tail + 1;
    while code.get(line).is_some_and(|text| text.trim().is_empty()) {
        line += 1;
    }
    if code
        .get(line)
        .is_some_and(|text| text.trim_start().starts_with('{'))
    {
        return braces.get(line).copied().flatten().unwrap_or(line);
    }
    end
}

// An `if` that hands over to an `else` runs to the end of every branch it names, wherever that branch starts.
fn else_tail(code: &[String], braces: &[Option<usize>], close: usize) -> usize {
    let mut end = close;
    loop {
        // The hand-over stands on the closing brace's own line, or on the first line of code after it.
        let branch = if hands_over(&code[end]) {
            end
        } else {
            let Some(next) = next_code(code, end + 1) else {
                return end;
            };
            if !hands_over(&code[next]) {
                return end;
            }
            // A bare `else` hands over to the body written under it.
            if code[next].trim() == "else" {
                next_code(code, next + 1).unwrap_or(next)
            } else {
                next
            }
        };
        let Some(closed) = braces.get(branch).copied().flatten() else {
            return end;
        };
        // A whole `if` on one line has no further branch to reach.
        if closed <= end {
            return end;
        }
        end = closed;
    }
}

// Whether a line hands over to an `else`, either as its own word or right after a closing brace.
fn hands_over(line: &str) -> bool {
    let line = line.trim_start();
    let Some(at) = line.find("else") else {
        return false;
    };
    let before = line[..at].trim_end();
    let opens = at == 0 || before.ends_with('}');
    let after = &line[at + "else".len()..];
    opens && (after.is_empty() || after.starts_with([' ', '(', '{']))
}

// The next line that holds any code, blank lines passed over.
fn next_code(code: &[String], from: usize) -> Option<usize> {
    (from..code.len()).find(|at| !code[*at].trim().is_empty())
}

// A piece's own header, folded over the lines it continues onto, the body's opener dropped.
fn header_text(code: &[String], raw: &[&str], at: usize, close: usize, kind: &str) -> String {
    // A call's own line names it whole; only a definition spelled `function` or `\(` runs its signature over several lines.
    if kind != "function" {
        return raw
            .get(at)
            .map_or(String::new(), |line| line.trim().to_string());
    }
    let mut text: Vec<&str> = Vec::new();
    let mut depth = 0i64;
    for line in at..=close {
        let Some(code_line) = code.get(line) else {
            break;
        };
        let piece = code_line.trim();
        // The body's opener ends the header; what a signature holds after it is the body.
        let cut = piece.find('{');
        let head = cut.map_or(piece, |at| piece[..at].trim_end()).trim();
        if !head.is_empty() {
            text.push(head);
        }
        if cut.is_some() {
            break;
        }
        for ch in code_line.chars() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
        }
        // The signature closes with the paren that brings the depth back to nought.
        if depth <= 0 {
            break;
        }
    }
    tidy(&text.join(" "))
}

// A header's own spacing: one space where a fold met the text, none inside the signature's brackets.
fn tidy(text: &str) -> String {
    let joined: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let opened = joined.replace("( ", "(").replace(" )", ")");
    opened
        .replace(" ,", ",")
        .replace("[ ", "[")
        .replace(" ]", "]")
}
