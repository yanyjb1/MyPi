//! The kind dispatch: one node in, rows out. Grouping and pairing live in `block`.

use ratatui::text::Line;

use super::super::block::Deferred;
use super::super::theme::{HistoryTheme, Token};
use super::super::{BlockKind, Streaming, render_block};
use super::glyphs;
use super::notice::notice_block;
use super::tool_card;
use super::tools::ToolOutcome;
use crate::server::storage::BlockKind as Kind;
use crate::ui::session::BlockRef;

fn push_block(
    out: &mut Vec<Line<'static>>,
    deferred: &mut Vec<Deferred>,
    (rows, mut d): (Vec<Line<'static>>, Vec<Deferred>),
) {
    let base = out.len();
    out.extend(rows);
    for x in &mut d {
        x.before += base;
    }
    deferred.extend(d);
}

pub(crate) fn single_node(
    block: BlockRef<'_>,
    paired: Option<BlockRef<'_>>,
    t: &HistoryTheme,
    thinking: bool,
    tool: bool,
    placeholder: &str,
    details: Option<&serde_json::Value>,
    width: usize,
    defer: bool,
    cwd: &str,
) -> (Vec<Line<'static>>, Vec<Deferred>) {
    match block.kind() {
        Kind::User => render_block(
            BlockKind::User,
            block.content().map(String::as_str).unwrap_or_default(),
            Streaming::Final,
            width,
            t,
            defer,
        ),
        Kind::Assistant => render_block(
            BlockKind::Assistant,
            block.content().map(String::as_str).unwrap_or_default(),
            Streaming::Final,
            width,
            t,
            defer,
        ),
        Kind::Reasoning => {
            if thinking {
                render_block(
                    BlockKind::Reasoning,
                    block.content().map(String::as_str).unwrap_or_default(),
                    Streaming::Final,
                    width,
                    t,
                    defer,
                )
            } else {
                (
                    vec![Line::from(t.fg(placeholder.to_string(), Token::SystemText))],
                    Vec::new(),
                )
            }
        }
        Kind::ToolRequest => tool_node(block, paired, t, tool, details, width, defer, cwd),
        Kind::ToolResult => (Vec::new(), Vec::new()),
    }
}

fn tool_node(
    block: BlockRef<'_>,
    paired: Option<BlockRef<'_>>,
    t: &HistoryTheme,
    tool: bool,
    details: Option<&serde_json::Value>,
    width: usize,
    defer: bool,
    cwd: &str,
) -> (Vec<Line<'static>>, Vec<Deferred>) {
    let name = block.name().map(String::as_str).unwrap_or_default();
    let args = block.args().map(String::as_str).unwrap_or_default();
    let text = block.text().map(String::as_str).unwrap_or_default();
    let first = block.first().unwrap_or(false);
    let outcome = paired.map(|r| outcome_of(r, details));
    let mut out = Vec::new();
    let mut deferred = Vec::new();

    if first && !text.trim().is_empty() {
        push_block(
            &mut out,
            &mut deferred,
            render_block(
                BlockKind::Assistant,
                text,
                Streaming::Final,
                width,
                t,
                defer,
            ),
        );
        out.push(super::super::block::block_gap());
    }

    if name == "todo" {
        out.extend(todo_marker(outcome.as_ref(), t));
        return (out, deferred);
    }
    if name == "read" || name == "cd" {
        out.extend(match name {
            "read" => super::tools::read_line(args, outcome.as_ref(), cwd, t),
            _ => super::tools::cd_line(args, outcome.as_ref(), cwd, t),
        });
        return (out, deferred);
    }
    out.extend(tool_card(name, args, outcome.as_ref(), t, tool, width));
    (out, deferred)
}

fn outcome_of<'a>(result: BlockRef<'a>, details: Option<&'a serde_json::Value>) -> ToolOutcome<'a> {
    ToolOutcome {
        text: result.result().map(String::as_str).unwrap_or_default(),
        ok: result.ok().unwrap_or(false),
        details,
        duration_ms: result.duration_ms(),
    }
}

fn todo_marker(outcome: Option<&ToolOutcome<'_>>, t: &HistoryTheme) -> Vec<Line<'static>> {
    let (glyph, tok) = match outcome {
        Some(o) if !o.ok => (glyphs::ERR, Token::Err),
        _ => (glyphs::PENDING, Token::ToolEdgePending),
    };
    let mut out = vec![Line::from(vec![t.fg(glyph, tok), t.fg(" Todo", tok)])];
    if let Some(o) = outcome.filter(|o| !o.ok) {
        for part in o.text.split('\n').filter(|s| !s.trim().is_empty()) {
            out.push(Line::from(t.fg(format!("  {part}"), Token::Err)));
        }
    }
    out
}

pub(crate) fn tool_output_rows(text: &str, t: &HistoryTheme, width: usize) -> Vec<Line<'static>> {
    super::super::block::wrap_rows(
        super::tools::output_lines(text, LIVE_TOOL_LINES, false, false, t).into_iter(),
        width,
    )
}

pub(crate) const LIVE_TOOL_LINES: usize = 12;
