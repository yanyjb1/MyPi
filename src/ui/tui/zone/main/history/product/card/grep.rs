//! grep: the query and its counts in the header, the answer's own sections below it.

use super::super::super::prefetch::color::code;
use super::super::super::row::{Lead, Row, Run};
use super::{Section, ToolBlock, ToolOutcome, ToolRenderer, error_body, partial};
use crate::ui::tui::theme::fixed;

/// The gap between a row's columns and its text.
const GAP: &str = "  ";

pub(crate) struct Grep;

impl ToolRenderer for Grep {
    fn label(&self, _b: &ToolBlock<'_>) -> Option<String> {
        Some("Grep".to_string())
    }

    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(query) = partial::text(b.args, "query").filter(|q| !q.trim().is_empty()) {
            out.push(format!("{:?}", one_line(&query)));
        }
        if let Some(o) = b.result {
            out.extend(counts(o.text));
        }
        out
    }

    fn call_body(&self, _b: &ToolBlock<'_>) -> Vec<Row> {
        Vec::new()
    }

    /// The head row's counts stand in for a line total.
    fn result_measure(&self, _o: ToolOutcome<'_>) -> Option<String> {
        None
    }

    fn result_sections(&self, _b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        if o.ok == Some(false) {
            return vec![Section {
                label: None,
                lines: error_body(&o.failure_text()),
            }];
        }
        vec![Section {
            label: None,
            lines: rows(o.text),
        }]
    }
}

fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

// is. Read by field name, so the tool's own order costs nothing here.
fn counts(text: &str) -> Vec<String> {
    let Some(stats) = text.lines().next().and_then(head_stats) else {
        return Vec::new();
    };
    stats
        .split(", ")
        .map(str::to_string)
        .filter(|field| field != "literal" && field != "regex")
        .take(4)
        .collect()
}

/// What follows the query on the head row. The split is on the last `": "`, since the query itself
/// may hold one.
fn head_stats(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("grep \"")?;
    rest.rsplit_once("\": ").map(|(_, stats)| stats)
}

/// Every row of the answer but its head.
fn rows(text: &str) -> Vec<Row> {
    let mut head = true;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if head {
            head = false;
            if head_stats(line).is_some() {
                continue;
            }
        }
        out.push(row(line));
    }
    out
}

/// One row, colored by what it holds.
fn row(line: &str) -> Row {
    if let Some(rest) = line.trim_start().strip_prefix('[') {
        return notice(line, rest);
    }
    if line.starts_with("DOWNSHIFTED") {
        return Row::of(Lead::None, vec![Run::fixed(line.to_string(), fixed::WARN)]);
    }
    if line.starts_with("==> ") {
        return Row::of(
            Lead::None,
            vec![Run::fixed(line.to_string(), fixed::TOOL_PATH)],
        );
    }
    if line.trim_start().starts_with("in: ") {
        return Row::of(Lead::None, vec![Run::fixed(line.to_string(), fixed::DIM)]);
    }
    if let Some((head, body)) = barred(line) {
        return columns(Run::fixed(head, fixed::DIM), body);
    }
    if let Some((head, body)) = spaced(line) {
        let ink = match head.contains(".rs") {
            true => fixed::TOOL_PATH,
            false => fixed::DIM,
        };
        return columns(Run::fixed(head.to_string(), ink), body);
    }
    if line.starts_with("src/") {
        return Row::of(
            Lead::None,
            vec![Run::fixed(line.to_string(), fixed::TOOL_PATH)],
        );
    }
    Row::of(Lead::None, vec![Run::plain(line.to_string())])
}

/// A bracketed row: a section's own title, a scope warning, or what a cap left out.
fn notice(line: &str, rest: &str) -> Row {
    let titled = ["[type] ", "[module] ", "[x-ray] ", "[hits] "]
        .iter()
        .any(|title| line.starts_with(title));
    if titled {
        return Row::of(
            Lead::None,
            vec![Run::fixed(line.to_string(), fixed::TOOL_TITLE).bold()],
        );
    }
    let capped =
        rest.starts_with("scope]") || line.starts_with("[+") || line.starts_with("DOWNSHIFTED");
    let ink = match capped {
        true => fixed::WARN,
        false => fixed::DIM,
    };
    Row::of(Lead::None, vec![Run::fixed(line.to_string(), ink)])
}

/// A source row: the gutter of line numbers before its bar, and the line itself after it. The
/// skeleton puts a space on each side of its bar; a hit's own line puts none, so the source keeps
/// its indentation.
fn barred(line: &str) -> Option<(String, &str)> {
    let (head, tail) = line.split_at(line.find('|')?);
    if !head
        .chars()
        .all(|c| c == ' ' || c == '.' || c.is_ascii_digit())
    {
        return None;
    }
    let body = tail.get(1..)?;
    if head.trim().is_empty() {
        return Some((head.to_string(), body));
    }
    match head.strip_suffix(' ') {
        Some(trimmed) => Some((trimmed.to_string(), body.strip_prefix(' ').unwrap_or(body))),
        None => Some((head.to_string(), body)),
    }
}

/// An indented row in two columns: a path and its signature, a span and its signature, a file and
/// its count. The left column is what says so: a `.rs` path or a span.
fn spaced(line: &str) -> Option<(&str, &str)> {
    let indent = line.len() - line.trim_start().len();
    if indent == 0 {
        return None;
    }
    let (head, _) = line[indent..].split_once(GAP)?;
    let span = head.chars().all(|c| c == '.' || c.is_ascii_digit());
    if !span && !head.contains(".rs") {
        return None;
    }
    let end = indent + head.len();
    Some((&line[..end], line[end..].trim_start_matches(' ')))
}

/// The columns before a row's text, then the text in its own colors.
fn columns(head: Run, body: &str) -> Row {
    if body.is_empty() {
        return Row::of(Lead::None, vec![head]);
    }
    let mut runs = vec![head, Run::fixed(GAP, fixed::DIM)];
    let colored = code::line(body, Some("rs"));
    if colored.runs.is_empty() {
        runs.push(Run::plain(body.to_string()));
    } else {
        runs.extend(colored.runs);
    }
    Row::of(Lead::None, runs)
}
