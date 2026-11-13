//! read, write and cd: the one-line forms, the numbered body, the preview.

use ratatui::style::Modifier;
use serde_json::Value;

use super::super::super::prefetch::color::code;
use super::super::super::row::{Ink, Lead, Row, Run};
use super::frame::{GUTTER_MIN, gutter_of, markdown_numbered, wrap_rows};
use super::{
    State, ToolBlock, ToolOutcome, ToolRenderer, arg_str, expand_tabs, head, indentation,
    item_error, partial, path_of, relativize,
};
use crate::ui::tui::theme::fixed;

pub(crate) struct FileTools;

impl ToolRenderer for FileTools {
    fn path(&self, b: &ToolBlock<'_>) -> Option<String> {
        path_of(b)
    }

    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(content) = partial::text(b.args, "content")
            && !content.is_empty()
        {
            out.push(format!("{} lines", content.lines().count()));
        }
        out
    }

    fn call_body(&self, b: &ToolBlock<'_>) -> Vec<Row> {
        write_preview(b)
    }
}

pub(crate) fn read_line(args: &str, result: Option<ToolOutcome<'_>>, cwd: &str) -> Vec<Row> {
    let files = partial::items(args, "files");
    let mut out = Vec::new();
    if files.is_empty() {
        out.push(Row::of(Lead::None, head(State::of(result), "Read")));
        return out;
    }
    for (index, item) in files.iter().enumerate() {
        let mut runs = head(State::of_item(result, index), "Read");
        let path = partial::field(item, "path").unwrap_or_default();
        if !path.trim().is_empty() {
            let mut target = relativize(cwd, &path);
            if let Some(span) = read_range(item) {
                target.push_str(&span);
            }
            runs.push(Run::fixed(format!(" {target}"), fixed::TOOL_PATH));
        }
        out.push(Row::of(Lead::None, runs));
        out.extend(item_error(result, index));
    }
    out
}

pub(crate) fn cd_line(args: &str, result: Option<ToolOutcome<'_>>, cwd: &str) -> Vec<Row> {
    let state = State::of(result);
    let landed = result
        .and_then(|o| o.details)
        .filter(|d| d.get("kind").and_then(|k| k.as_str()) == Some("cwd"))
        .and_then(|d| d.get("path").and_then(|p| p.as_str()).map(String::from));
    let want = landed.or_else(|| arg_str(args, "path"));
    let base = result
        .and_then(|o| o.details)
        .and_then(|d| d.get("from").and_then(|p| p.as_str()).map(String::from))
        .unwrap_or_else(|| cwd.to_string());
    let mut runs = vec![
        Run::with(state.glyph(), Some(state.glyph_ink()), Modifier::empty()),
        Run::fixed(" Cd", fixed::TOOL_TITLE).bold(),
    ];
    if let Some(p) = want.filter(|p| !p.trim().is_empty()) {
        let ink = match state {
            State::Err => Ink::Fixed(fixed::ERR),
            _ => Ink::Fixed(fixed::TOOL_PATH),
        };
        runs.push(Run::with(
            format!(" {}", relativize(&base, &p)),
            Some(ink),
            Modifier::empty(),
        ));
    }
    vec![Row::of(Lead::None, runs)]
}

fn read_range(item: &Value) -> Option<String> {
    let offset = item.get("offset").and_then(|o| o.as_u64());
    let limit = item.get("limit").and_then(|l| l.as_u64());
    if offset.is_none() && limit.is_none() {
        return None;
    }
    let start = offset.unwrap_or(1);
    Some(match limit {
        Some(n) => format!(":{}-{}", start, start + n.saturating_sub(1)),
        None => format!(":{start}"),
    })
}

pub(crate) fn read_body(o: ToolOutcome<'_>) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    let mut numbered: Vec<(usize, String)> = Vec::new();
    for line in o.text.lines() {
        if let Some(path) = line.strip_prefix("==> ") {
            flush_numbered(&mut numbered, &mut out);
            out.push(Row::of(
                Lead::None,
                vec![Run::fixed(format!("==> {path}"), fixed::TOOL_PATH)],
            ));
            continue;
        }
        let parsed = line
            .trim_start()
            .split_once('\t')
            .and_then(|(n, text)| n.parse::<usize>().ok().map(|n| (n, expand_tabs(text))));
        match parsed {
            Some((n, text)) => numbered.push((n, text)),
            None => {
                flush_numbered(&mut numbered, &mut out);
                out.push(Row::of(Lead::None, vec![Run::fixed(line, fixed::MUTED)]));
            }
        }
    }
    flush_numbered(&mut numbered, &mut out);
    out
}

fn flush_numbered(numbered: &mut Vec<(usize, String)>, out: &mut Vec<Row>) {
    if numbered.is_empty() {
        return;
    }
    let width = numbered
        .iter()
        .map(|(n, _)| n.to_string().len())
        .max()
        .unwrap_or(1)
        .max(GUTTER_MIN);
    for (n, text) in numbered.drain(..) {
        let mut runs = vec![Run::fixed(format!("{n:>width$}│"), fixed::DIM)];
        let hl = code::line(&text, None);
        if hl.runs.is_empty() {
            runs.push(Run::plain(text));
        } else {
            runs.extend(hl.runs);
        }
        out.push(Row::of(Lead::None, runs));
    }
}

/// How a written file's content is drawn, decided by the path it names.
enum Body {
    /// A document: the conversation's own markdown rendering, numbered by source line.
    Markdown,
    /// Plain text: numbered, indentation kept, long lines continued.
    Text,
    /// Source: colored line by line, never wrapped.
    Code,
}

fn body_of(path: &str) -> Body {
    // The alias table answers with a row's first spelling, so markdown comes back as `md`.
    match code::language_for_path(path) {
        Some("md") => Body::Markdown,
        Some(_) => Body::Code,
        None => Body::Text,
    }
}

pub(crate) fn write_preview(b: &ToolBlock<'_>) -> Vec<Row> {
    let content = partial::text(b.args, "content").unwrap_or_default();
    if content.trim().is_empty() {
        return Vec::new();
    }
    let path = partial::text(b.args, "path").unwrap_or_default();
    match body_of(&path) {
        Body::Markdown => markdown_numbered(&content, b.inner_width()),
        Body::Text => text_preview(&content, b.inner_width()),
        Body::Code => code_preview(&content, &path),
    }
}

/// Plain text: every source line numbered, its indentation kept, long lines continued.
fn text_preview(content: &str, width: usize) -> Vec<Row> {
    let gutter = gutter_of(content);
    let mut out: Vec<Row> = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let mut runs = vec![Run::fixed(format!("{:>gutter$} ", i + 1), fixed::DIM)];
        let (glyphs, body) = indentation(line);
        if !glyphs.is_empty() {
            runs.push(Run::with(
                glyphs,
                Some(Ink::Fixed(fixed::DIM)),
                Modifier::DIM,
            ));
        }
        runs.push(Run::plain(body));
        out.push(Row::of(Lead::None, runs));
    }
    wrap_rows(out, width)
}

/// Source: colored line by line, its own width the reader's business.
fn code_preview(content: &str, path: &str) -> Vec<Row> {
    let lang = code::language_for_path(path);
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let gutter = total.to_string().len().max(GUTTER_MIN);
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let mut runs = vec![Run::fixed(format!("{:>gutter$} ", i + 1), fixed::DIM)];
        let (glyphs, body) = indentation(line);
        if !glyphs.is_empty() {
            runs.push(Run::with(
                glyphs,
                Some(Ink::Fixed(fixed::DIM)),
                Modifier::DIM,
            ));
        }
        let painted = code::line(&body, lang);
        if painted.runs.is_empty() {
            runs.push(Run::plain(body));
        } else {
            runs.extend(painted.runs);
        }
        out.push(Row::of(Lead::None, runs));
    }
    out
}
