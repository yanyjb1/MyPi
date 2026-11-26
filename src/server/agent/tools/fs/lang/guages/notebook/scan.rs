// The notebook scan: which lines each cell of a `.ipynb` occupies, read as text and never parsed whole.

// What one cell of a notebook is, as its own brackets read.
pub(crate) struct Cell {
    // The cell's place in the `cells` array, 1-based, as ruff numbers it.
    pub(crate) index: usize,

    // Its own word: `code`, `markdown` or `raw`.
    pub(crate) kind: String,

    // The line its `{` opens on, 1-based.
    pub(crate) open: usize,

    // The line its `}` closes on, 1-based.
    pub(crate) close: usize,

    // The file line each row of its `source` stands on, 1-based and in row order; empty when the cell writes no source. One row is not one line: a source written as one string carries many rows on the line it stands on.
    pub(crate) rows: Vec<usize>,
}

// Every cell of one notebook, in the order the file writes them.
pub(crate) fn cells(text: &str) -> Vec<Cell> {
    let lines: Vec<&str> = text.lines().collect();
    let Some((open, close)) = cells_array(&lines) else {
        return Vec::new();
    };
    let mut out: Vec<Cell> = Vec::new();
    let mut at = open;
    while at <= close {
        let Some(start) = opening(&lines, at, close) else {
            break;
        };
        let Some(end) = matching(&lines, start, b'{', close) else {
            break;
        };
        out.push(Cell {
            index: out.len() + 1,
            kind: kind(&lines, start, end),
            open: start + 1,
            close: end + 1,
            rows: rows(&lines, start, end),
        });
        at = end + 1;
    }
    out
}

// The `cells` array, 0-based: the line its first cell stands on, and the line its `]` closes on.
fn cells_array(lines: &[&str]) -> Option<(usize, usize)> {
    let named = lines.iter().position(|line| key(line, "cells").is_some())?;
    let open = (named..lines.len()).find(|at| lines[*at].contains('['))?;
    let close = matching(lines, open, b'[', lines.len())?;
    Some((open + 1, close))
}

// The first line at or below `from` that opens an object.
fn opening(lines: &[&str], from: usize, end: usize) -> Option<usize> {
    (from..=end.min(lines.len().saturating_sub(1))).find(|at| lines[*at].contains('{'))
}

// The line the bracket opened on `open` finally closes on, counting brackets and stepping over quoted text.
fn matching(lines: &[&str], open: usize, opener: u8, end: usize) -> Option<usize> {
    let closer = if opener == b'[' { b']' } else { b'}' };
    let mut depth = 0isize;
    let mut quoted = false;
    let mut escaped = false;
    let last = end.min(lines.len().saturating_sub(1));
    for (at, line) in lines.iter().enumerate().take(last + 1).skip(open) {
        for byte in line.bytes() {
            if escaped {
                escaped = false;
                continue;
            }
            if quoted {
                match byte {
                    b'\\' => escaped = true,
                    b'"' => quoted = false,
                    _ => {}
                }
                continue;
            }
            match byte {
                b'"' => quoted = true,
                one if one == opener => depth += 1,
                one if one == closer => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(at);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

// One cell's own word, from the `cell_type` it writes; `raw` for a cell that names none.
fn kind(lines: &[&str], open: usize, close: usize) -> String {
    for line in &lines[open..=close] {
        let Some(at) = key(line, "cell_type") else {
            continue;
        };
        let Some((_, value)) = line[at + "\"cell_type\"".len()..].split_once(':') else {
            continue;
        };
        let word: String = value
            .trim()
            .trim_start_matches('"')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        if !word.is_empty() {
            return word;
        }
    }
    "raw".to_string()
}

// The file line of each row a cell's `source` holds, in row order: an array of one string per line, or one bare string holding them all on the key's own line.
fn rows(lines: &[&str], open: usize, close: usize) -> Vec<usize> {
    let Some((at, found)) =
        (open..=close).find_map(|at| key(lines[at], "source").map(|found| (at, found)))
    else {
        return Vec::new();
    };
    let Some((_, value)) = lines[at][found + "\"source\"".len()..].split_once(':') else {
        return Vec::new();
    };
    let value = value.trim_start();
    if !value.starts_with('[') {
        return match value.starts_with('"') {
            true => vec![at + 1; rows_of(value)],
            false => Vec::new(),
        };
    }
    let mut out: Vec<usize> = Vec::new();
    if let Some(first) = value.strip_prefix('[').map(str::trim_start)
        && first.starts_with('"')
    {
        out.extend(std::iter::repeat_n(at + 1, rows_of(first)));
    }
    let stop = matching(lines, at, b'[', close).unwrap_or(close);
    for (line, element) in lines.iter().enumerate().take(stop + 1).skip(at + 1) {
        let element = element.trim_start();
        if element.starts_with('"') {
            out.extend(std::iter::repeat_n(line + 1, rows_of(element)));
        }
    }
    out
}

// How many rows one JSON string literal carries: each `\n` escape closes a row, and the one it ends on closes the last.
fn rows_of(literal: &str) -> usize {
    let body = literal.strip_prefix('"').unwrap_or(literal);
    let mut closed = 0usize;
    let mut ends_row = false;
    let mut at = 0usize;
    let bytes = body.as_bytes();
    while at < bytes.len() {
        match bytes[at] {
            b'"' => break,
            b'\\' => {
                ends_row = bytes.get(at + 1) == Some(&b'n');
                closed += usize::from(ends_row);
                at += 2;
            }
            _ => {
                ends_row = false;
                at += 1;
            }
        }
    }
    match closed {
        0 => 1,
        _ if ends_row => closed,
        _ => closed + 1,
    }
}

// One row ruff counts inside a cell, as the file's own line; a row past the cell's last stands on its last line.
pub(crate) fn place(cell: &Cell, row: usize) -> usize {
    let last = cell.rows.last().copied().unwrap_or(cell.open);
    cell.rows
        .get(row.saturating_sub(1))
        .copied()
        .unwrap_or(last)
}

// Where one line names a JSON key, when it does: a quoted word that is not itself inside a quoted value, and that a colon follows. A cell's outputs quote whole texts, and a key is the one place its quotes stand unescaped.
fn key(line: &str, name: &str) -> Option<usize> {
    let quoted = format!("\"{name}\"");
    let mut from = 0usize;
    while let Some(at) = line[from..].find(&quoted) {
        let at = from + at;
        let escaped = at > 0 && line.as_bytes()[at - 1] == b'\\';
        let tail = line[at + quoted.len()..].trim_start();
        if !escaped && tail.starts_with(':') {
            return Some(at);
        }
        from = at + quoted.len();
    }
    None
}
