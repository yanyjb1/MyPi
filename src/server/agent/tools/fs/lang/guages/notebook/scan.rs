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

    // The line the cell's first `source` string stands on, 1-based; `None` when the cell writes none.
    pub(crate) source: Option<usize>,
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
            source: source(&lines, start, end),
        });
        at = end + 1;
    }
    out
}

// The `cells` array, 0-based: the line its first cell stands on, and the line its `]` closes on.
fn cells_array(lines: &[&str]) -> Option<(usize, usize)> {
    let named = lines.iter().position(|line| line.contains("\"cells\""))?;
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
        let Some(at) = line.find("\"cell_type\"") else {
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

// The line the cell's own source starts on, 1-based: the first string of its `source`, whether that is an array or one bare string.
fn source(lines: &[&str], open: usize, close: usize) -> Option<usize> {
    for at in open..=close {
        let Some(found) = lines[at].find("\"source\"") else {
            continue;
        };
        let rest = &lines[at][found + "\"source\"".len()..];
        let Some((_, value)) = rest.split_once(':') else {
            continue;
        };
        let value = value.trim_start();
        if !value.starts_with('[') {
            return value.starts_with('"').then_some(at + 1);
        }
        if value[1..].trim_start().starts_with('"') {
            return Some(at + 1);
        }
        return (at + 1..=close)
            .find(|line| lines[*line].trim_start().starts_with('"'))
            .map(|line| line + 1);
    }
    None
}

// One row ruff counts inside a cell, as the file's own line.
pub(crate) fn place(cell: &Cell, row: usize) -> usize {
    cell.source
        .unwrap_or(cell.open)
        .saturating_add(row)
        .saturating_sub(1)
}
