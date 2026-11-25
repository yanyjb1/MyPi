// The Markdown sorter: which lines are key, what each piece is, and how far it reaches.

use std::path::Path;

use super::LANGUAGE;
use crate::server::agent::tools::fs::lang::{Line, Table};

// One file's line table.
pub(crate) fn table(_path: &Path, text: &str) -> Table {
    let raw: Vec<&str> = text.lines().collect();
    let mut out = Table::new(raw.len(), &LANGUAGE);
    let mut headings: Vec<(usize, usize)> = Vec::new();
    let mut at = front(&raw, &mut out);
    while at < raw.len() {
        if let Some((mark, run)) = fence(raw[at]) {
            at = fenced(&raw, &mut out, at, mark, run);
        } else if let Some(level) = heading(raw[at]) {
            headings.push((at, level));
            at += 1;
        } else if quoted(raw[at]) {
            at = quote(&raw, &mut out, at);
        } else if let Some(tag) = html(raw[at]) {
            at = block(&raw, &mut out, at, &tag);
        } else if marker(raw[at]) {
            at = listed(&raw, &mut out, at);
        } else {
            at += 1;
        }
    }
    sections(&raw, &mut out, &headings);
    out
}

// What a piece of this file is: `heading`, `fence`, `list`, `quote`, `html`, `front`.
pub(crate) fn shape(line: &Line) -> Option<&str> {
    line.label.as_deref()
}

// The front matter's pair of `---` at the file's head; the line the rest of the file starts on. An unclosed `---` is a thematic break, which carries nothing.
fn front(raw: &[&str], table: &mut Table) -> usize {
    if raw.first().map(|line| line.trim_end()) != Some("---") {
        return 0;
    }
    for (at, line) in raw.iter().enumerate().skip(1) {
        if line.trim_end() == "---" {
            table.span(0, at, Some("front".to_string()), Some("---".to_string()));
            return at + 1;
        }
    }
    0
}

// The fence this line opens: its mark, and how many marks it runs.
pub(super) fn fence(line: &str) -> Option<(char, usize)> {
    let text = line.trim_start();
    let Some(mark @ ('`' | '~')) = text.chars().next() else {
        return None;
    };
    let run = text.chars().take_while(|one| *one == mark).count();
    (run >= 3).then_some((mark, run))
}

// Whether this line closes a fence opened with `run` marks of `mark`, and carries nothing else.
pub(super) fn closes(line: &str, mark: char, run: usize) -> bool {
    let text = line.trim();
    let marks = text.chars().take_while(|one| *one == mark).count();
    marks >= run && marks == text.chars().count()
}

// A fenced block: its two fence lines are key, and nothing inside it is read as structure.
fn fenced(raw: &[&str], table: &mut Table, open: usize, mark: char, run: usize) -> usize {
    let info = info(raw[open], mark);
    if let Some(close) = (open + 1..raw.len()).find(|at| closes(raw[*at], mark, run)) {
        table.span(open, close, Some("fence".to_string()), info);
        close + 1
    } else {
        table.span(open, open, Some("fence".to_string()), info);
        open + 1
    }
}

// The word a fence names itself with: ```` ```rust ```` reads `rust`.
fn info(line: &str, mark: char) -> Option<String> {
    let rest = line.trim().trim_start_matches(mark).trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

// A heading's level, when this line is one: one to six marks, then a space or the line's end.
fn heading(line: &str) -> Option<usize> {
    let text = line.trim_start();
    let level = text.chars().take_while(|one| *one == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &text[level..];
    (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')).then_some(level)
}

// Whether this line is quoted.
fn quoted(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

// A run of quoted lines: its first and its last are key.
fn quote(raw: &[&str], table: &mut Table, open: usize) -> usize {
    let mut at = open;
    while at < raw.len() && quoted(raw[at]) {
        at += 1;
    }
    let first = raw[open].trim().to_string();
    table.span(open, at - 1, Some("quote".to_string()), Some(first));
    at
}

// The tag this line opens an HTML block with; a line whose `<` names no tag is prose.
fn html(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix('<')?;
    let tag: String = rest
        .chars()
        .take_while(|one| one.is_ascii_alphanumeric() || *one == '-')
        .collect();
    tag.chars()
        .next()
        .is_some_and(|one| one.is_ascii_alphabetic())
        .then_some(tag)
}

// One HTML block: the line naming its tag and the line closing it. A tag that never closes owns only its own line.
fn block(raw: &[&str], table: &mut Table, open: usize, tag: &str) -> usize {
    let closing = format!("</{tag}");
    let close = (open..raw.len())
        .find(|at| raw[*at].contains(closing.as_str()))
        .unwrap_or(open);
    let exact = format!("<{tag}>");
    table.span(open, close, Some("html".to_string()), Some(exact));
    close + 1
}

// Whether this line starts a list item: a bullet, or a number with `.` or `)`, then a space.
fn marker(line: &str) -> bool {
    let text = line.trim_start();
    let mut rest = text.chars();
    let Some(first) = rest.next() else {
        return false;
    };
    let after = if first.is_ascii_digit() {
        let digits = text.chars().take_while(char::is_ascii_digit).count();
        let mut tail = text[digits..].chars();
        match tail.next() {
            Some('.' | ')') => tail.next(),
            _ => return false,
        }
    } else if matches!(first, '-' | '*' | '+') {
        rest.next()
    } else {
        return false;
    };
    matches!(after, Some(' ' | '\t'))
}

// One list: its first and its last line are key, and outer item lines that stand alone when items nest.
fn listed(raw: &[&str], table: &mut Table, open: usize) -> usize {
    let base = indent(raw[open]);
    let mut outer: Vec<usize> = Vec::new();
    let mut nested = false;
    let mut body = open;
    let mut at = open;
    while at < raw.len() {
        let line = raw[at];
        if line.trim().is_empty() {
            // A blank line inside a list keeps it open; what follows still has to be an item.
            at += 1;
            continue;
        }
        if !marker(line) {
            break;
        }
        let depth = indent(line);
        if depth < base {
            break;
        }
        if depth > base {
            nested = true;
        } else {
            outer.push(at);
        }
        body = at;
        at += 1;
    }
    let first = raw[open].trim().to_string();
    table.span(open, body, Some("list".to_string()), Some(first));
    if nested {
        for item in outer {
            // An item line already heading the run's own piece keeps it; only blank lines get one.
            if !table.line(item).is_some_and(Line::head) {
                table.span(item, item, None, None);
            }
        }
    }
    at
}

// How far this line is indented: a list's nesting is its indentation and nothing else.
fn indent(line: &str) -> usize {
    line.chars()
        .take_while(|one| *one == ' ' || *one == '\t')
        .count()
}

// Every heading's section: its heading line is key, its reach is remembered for the tools that ask which section a line sits in. The section's own last line is prose, and prose gets no number.
fn sections(raw: &[&str], table: &mut Table, headings: &[(usize, usize)]) {
    let last = raw.len().saturating_sub(1);
    for (index, (at, level)) in headings.iter().enumerate() {
        let end = headings[index + 1..]
            .iter()
            .find(|(_, deeper)| deeper <= level)
            .map_or(last, |(next, _)| next.saturating_sub(1));
        let end = trail(raw, *at, end);
        let exact = raw[*at].trim().to_string();
        table.reach(*at, end, Some("heading".to_string()), Some(exact));
    }
}

// The last line of a range that carries something: trailing blank lines are nobody's section.
fn trail(raw: &[&str], from: usize, end: usize) -> usize {
    let mut end = end.min(raw.len().saturating_sub(1));
    while end > from && raw[end].trim().is_empty() {
        end -= 1;
    }
    end
}
