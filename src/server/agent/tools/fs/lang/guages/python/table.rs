// The Python sorter: which lines are key, what each piece is, and where its block ends.

use std::path::Path;

use super::LANGUAGE;
use super::scan;
use crate::server::agent::tools::fs::lang::{Line, Table};

// The suffixes this sorter answers to.
pub(crate) const SUFFIXES: &[&str] = &["py", "pyi"];

// Words that begin a piece: a definition, an import, or the script's own guard.
const HEADS: &[&str] = &["def", "class", "import", "from", "if"];

// Words that open a block inside a long definition.
const CONTROL: &[&str] = &[
    "if", "elif", "else", "for", "while", "try", "except", "finally", "with", "match",
];

// A definition at least this long hands out key lines inside its own body.
const LONG: usize = 15;

// One file's line table.
pub(crate) fn table(_path: &Path, text: &str) -> Table {
    let scanned = scan::strip(text);
    let mut table = Table::new(scanned.code.len(), &LANGUAGE);
    let indent = indents(&scanned);
    pieces(&scanned, &indent, &mut table);
    control(&scanned, &indent, &mut table);
    table
}

// The kind of piece a line opens: the head word of its label, so `async def` reads `def`.
pub(crate) fn shape(line: &Line) -> Option<&str> {
    let label = line.label.as_deref()?;
    label.split_whitespace().find(|word| HEADS.contains(word))
}

// How far each line is pushed in; `None` for a blank line, a comment, or a folded continuation.
fn indents(scanned: &scan::Scanned) -> Vec<Option<usize>> {
    scanned
        .code
        .iter()
        .enumerate()
        .map(|(at, code)| {
            if scanned.carried.get(at).copied().unwrap_or(false) {
                return None;
            }
            if code.trim().is_empty() {
                return None;
            }
            Some(code.len() - code.trim_start().len())
        })
        .collect()
}

// Every definition, import, script guard and decorator block.
fn pieces(scanned: &scan::Scanned, indent: &[Option<usize>], table: &mut Table) {
    let sig: Vec<usize> = (0..indent.len())
        .filter(|at| indent[*at].is_some())
        .collect();
    let mut k = 0usize;
    while k < sig.len() {
        let at = sig[k];
        let code = scanned.code[at].trim_start();
        if code.starts_with('@') {
            // Decorators stand together: the run's first and last lines are key, and it binds nothing.
            let mut last = at;
            while k + 1 < sig.len() && scanned.code[sig[k + 1]].trim_start().starts_with('@') {
                k += 1;
                last = sig[k];
            }
            table.span(at, last, None, None);
            k += 1;
            continue;
        }
        let Some(kind) = head_of(code) else {
            k += 1;
            continue;
        };
        let depth = indent[at].unwrap_or(0);
        let stop = end_of(&sig[k..], at, depth, indent);
        let header = header_text(scanned, at);
        table.span(at, stop, Some(label_of(code, kind)), Some(header));
        k += 1;
    }
}

fn end_of(rest: &[usize], at: usize, depth: usize, indent: &[Option<usize>]) -> usize {
    let mut last = at;
    for line in rest.iter().copied().skip(1) {
        if indent[line].is_some_and(|own| own <= depth) {
            break;
        }
        last = line;
    }
    last
}

// The head word of a statement, once nothing else stands in front of it.
fn head_of(code: &str) -> Option<&'static str> {
    let first = code.split_whitespace().next()?;
    if first == "async" {
        // `async def` and `async for` both carry their own head word behind it.
        let second = code.split_whitespace().nth(1)?;
        return HEADS
            .iter()
            .copied()
            .find(|head| *head == second && *head == "def");
    }
    let word = first.trim_end_matches('(').trim_end_matches(':');
    HEADS.iter().copied().find(|head| *head == word)
}

// The label a piece carries: the words in front of its head, then the head itself.
fn label_of(code: &str, kind: &'static str) -> String {
    if kind == "def" && code.starts_with("async ") {
        return "async def".to_string();
    }
    kind.to_string()
}

// A piece's header, folded lines joined and the block's own colon dropped.
fn header_text(scanned: &scan::Scanned, at: usize) -> String {
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
    let joined = parts.join(" ");
    let joined = joined.trim().trim_end_matches(':').trim_end();
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
}

// The blocks inside a long definition: its body's own first level, when the definition is long.
fn control(scanned: &scan::Scanned, indent: &[Option<usize>], table: &mut Table) {
    let pieces: Vec<(usize, usize)> = table
        .pieces()
        .into_iter()
        .filter(|(_, line)| shape(line) == Some("def"))
        .filter_map(|(at, _)| table.end(at).map(|end| (at, end)))
        .collect();
    for (at, end) in pieces {
        if end <= at + LONG {
            continue;
        }
        let Some(depth) = indent.get(at).copied().flatten() else {
            continue;
        };
        let body = (at + 1..=end)
            .filter_map(|n| indent.get(n).copied().flatten().map(|own| (n, own)))
            .filter(|(_, own)| *own > depth)
            .min_by_key(|(_, own)| *own)
            .map(|(_, own)| own);
        let Some(body) = body else {
            continue;
        };
        for n in at + 1..=end {
            if indent.get(n).copied().flatten() != Some(body) || table.key(n) {
                continue;
            }
            if !control_head(scanned.code[n].trim_start()) {
                continue;
            }
            let stop = end_of(&(n..=end).collect::<Vec<_>>(), n, body, indent);
            table.span(n, stop, None, None);
        }
    }
}

// Whether a body line opens a block of its own.
fn control_head(code: &str) -> bool {
    let first = code.split_whitespace().next().unwrap_or("");
    let word = first.trim_end_matches(':');
    CONTROL.contains(&word)
}
