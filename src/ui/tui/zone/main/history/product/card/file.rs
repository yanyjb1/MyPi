//! read, write and cd: the one-line forms, the numbered body, the preview.

use ratatui::style::Modifier;
use serde_json::Value;

use super::super::super::prefetch::color::code;
use super::super::super::row::{Ink, Lead, Row, Run};
use super::{
    State, ToolBlock, ToolOutcome, ToolRenderer, arg_str, expand_tabs, partial, path_meta,
    relativize,
};
use crate::ui::tui::theme::fixed;

const WRITE_GUTTER_MIN_WIDTH: usize = 3;

pub(crate) struct FileTools;

impl ToolRenderer for FileTools {
    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        let mut out = path_meta(b);
        if b.name == "write"
            && let Some(content) = partial::text(b.args, "content")
            && !content.is_empty()
        {
            out.push(format!("{} lines", content.lines().count()));
        }
        if b.name == "read"
            && let Some(o) = b.result
            && let Some(d) = o.details
        {
            let offset = d.get("offset").and_then(|v| v.as_u64()).unwrap_or(1);
            let lines = d.get("lines").and_then(|v| v.as_u64()).unwrap_or(0);
            if offset > 1 && lines > 0 {
                let span = format!(":{}-{}", offset, offset + lines - 1);
                if let Some(first) = out.first_mut() {
                    first.push_str(&span);
                }
            }
        }
        out
    }

    fn call_body(&self, b: &ToolBlock<'_>) -> Vec<Row> {
        if b.name == "write" {
            write_preview(b)
        } else {
            Vec::new()
        }
    }
}

pub(crate) fn read_line(args: &str, result: Option<ToolOutcome<'_>>, cwd: &str) -> Vec<Row> {
    let state = State::of(result);
    let mut runs = vec![
        Run::with(state.glyph(), Some(state.glyph_ink()), Modifier::empty()),
        Run::fixed(" Read", fixed::TOOL_TITLE).bold(),
    ];
    let path = arg_str(args, "path").unwrap_or_default();
    if !path.trim().is_empty() {
        let mut target = relativize(cwd, &path);
        if let Some(span) = read_range(args) {
            target.push_str(&span);
        }
        runs.push(Run::fixed(format!(" {target}"), fixed::MUTED));
    }
    let mut out = vec![Row::of(Lead::None, runs)];
    if let Some(o) = result.filter(|o| o.ok == Some(false)) {
        for part in o.text.lines().filter(|s| !s.trim().is_empty()) {
            out.push(Row::of(
                Lead::None,
                vec![Run::fixed(format!("  {part}"), fixed::ERR)],
            ));
        }
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
            State::Ok => Ink::Fixed(fixed::MUTED),
            _ => Ink::Fixed(fixed::ERR),
        };
        runs.push(Run::with(
            format!(" {}", relativize(&base, &p)),
            Some(ink),
            Modifier::empty(),
        ));
    }
    vec![Row::of(Lead::None, runs)]
}

fn read_range(args: &str) -> Option<String> {
    let v: Value = serde_json::from_str(args).ok()?;
    let offset = v.get("offset").and_then(|o| o.as_u64());
    let limit = v.get("limit").and_then(|l| l.as_u64());
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
        let parsed = line.trim_start().split_once('\t').and_then(|(n, text)| {
            n.parse::<usize>()
                .ok()
                .map(|n| (n, text.replace('\t', "    ")))
        });
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
        .max(WRITE_GUTTER_MIN_WIDTH);
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

pub(crate) fn write_preview(b: &ToolBlock<'_>) -> Vec<Row> {
    let content = partial::text(b.args, "content").unwrap_or_default();
    if content.trim().is_empty() {
        return Vec::new();
    }
    let lang = code::language_for_path(&partial::text(b.args, "path").unwrap_or_default());
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let gutter = total.to_string().len().max(WRITE_GUTTER_MIN_WIDTH);
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let mut runs = vec![Run::fixed(format!("{:>gutter$} ", i + 1), fixed::DIM)];
        let body = expand_tabs(line);
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
