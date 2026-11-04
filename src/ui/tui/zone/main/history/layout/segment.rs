//! The segment model: the source split into runs that each render on their own.
//!
//! A segment is the smallest thing this front end paints and the only unit a re-layout or a
//! backwards walk can start from. Boundaries are decided on source lines.

/// What a run of source lines is, and therefore how it is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Ordinary prose: paragraphs, headings, rules.
    Text,
    /// A fenced code block, fence lines included.
    Code,
    /// A table: the header, its delimiter row, and the body rows.
    Table,
    /// A run of `>` lines.
    Quote,
    /// A list: its items and their indented continuations.
    List,
    /// The tail of a paragraph the line above hard-broke.
    Break,
}

/// One run of source lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub kind: Kind,
    /// The source as it stood, newlines included.
    pub source: String,
    /// The fence's language, for a code segment.
    pub lang: Option<String>,
}

/// Split one block's text into segments, in source order.
pub fn split(text: &str) -> Vec<Segment> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out: Vec<Segment> = Vec::new();
    let mut at = 0usize;
    let mut hard = false;
    while at < lines.len() {
        let line = lines[at];
        if let Some((lang, opener)) = fence(line) {
            let (end, closed) = fence_end(&lines, at + 1, opener);
            let stop = if closed { end + 1 } else { lines.len() };
            out.push(Segment {
                kind: Kind::Code,
                source: lines[at..stop].concat(),
                lang,
            });
            at = stop;
            hard = false;
            continue;
        }
        if table_head(&lines, at) {
            let mut end = at + 2;
            while end < lines.len() && lines[end].contains('|') {
                end += 1;
            }
            out.push(Segment {
                kind: Kind::Table,
                source: lines[at..end].concat(),
                lang: None,
            });
            at = end;
            hard = false;
            continue;
        }
        let kind = if quote(line) {
            Kind::Quote
        } else if item(line) {
            Kind::List
        } else if hard {
            Kind::Break
        } else {
            Kind::Text
        };
        let mut end = at + 1;
        while end < lines.len() && same(kind, &lines, end) {
            end += 1;
        }
        hard = hard_break(lines[end - 1]);
        out.push(Segment {
            kind,
            source: lines[at..end].concat(),
            lang: None,
        });
        at = end;
    }
    out
}

/// Whether the line keeps the run of this kind going.
fn same(kind: Kind, lines: &[&str], at: usize) -> bool {
    let line = lines[at];
    if fence(line).is_some() || table_head(lines, at) {
        return false;
    }
    match kind {
        Kind::Quote => quote(line),
        // A list keeps its indented continuations, and indented lines keep a paragraph.
        Kind::List => item(line) || indent(line),
        Kind::Text | Kind::Break => !quote(line) && !item(line) && !indent(line),
        _ => false,
    }
}

/// A fence opener: the marker and its language.
fn fence(line: &str) -> Option<(Option<String>, &'static str)> {
    let trimmed = line.trim_start();
    for marker in ["```", "~~~"] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            let lang = rest
                .split([' ', ',', '{'])
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            let lang = (!lang.is_empty()).then_some(lang);
            return Some((lang, marker));
        }
    }
    None
}

/// Where the fence closes: the index, and whether a closer was found.
fn fence_end(lines: &[&str], from: usize, marker: &str) -> (usize, bool) {
    for (i, line) in lines.iter().enumerate().skip(from) {
        if line.trim_start().starts_with(marker) {
            return (i, true);
        }
    }
    (lines.len(), false)
}

/// A quote line.
fn quote(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

/// A list item.
fn item(line: &str) -> bool {
    let trimmed = line.trim_start();
    let mut chars = trimmed.chars();
    match chars.next() {
        Some('-') | Some('*') | Some('+') => trimmed[1..].starts_with(' '),
        Some(c) if c.is_ascii_digit() => {
            let rest = trimmed.trim_start_matches(|c: char| c.is_ascii_digit());
            rest.starts_with(". ") || rest.starts_with(") ")
        }
        _ => false,
    }
}

/// A line that belongs to the line above rather than starting its own.
fn indent(line: &str) -> bool {
    line.starts_with(' ') || line.starts_with('\t')
}

/// Whether this line ends with a markdown hard break.
fn hard_break(line: &str) -> bool {
    let body = line.trim_end_matches('\n');
    body.ends_with("  ") || body.ends_with('\\')
}

/// A table header: this line has cells, and the next is a delimiter row.
fn table_head(lines: &[&str], at: usize) -> bool {
    let Some(line) = lines.get(at) else {
        return false;
    };
    if !line.contains('|') {
        return false;
    }
    let Some(next) = lines.get(at + 1) else {
        return false;
    };
    next.chars()
        .all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t' | '\n'))
        && next.contains('-')
}
