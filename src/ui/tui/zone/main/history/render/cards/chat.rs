//! The tool card and the live tail: what a running turn draws.

use ratatui::text::Line;
use serde_json::Value;

use super::super::super::shape::{Block, Kind};
use super::super::Opts;
use super::super::block::block_gap;
use super::super::paint;
use super::Card;
use super::tool_card;
use super::tools::{self, ToolOutcome};
use crate::ui::tui::text::glyphs;
use crate::ui::tui::theme::{HistoryTheme, Token};

/// One tool exchange, and the assistant text that came before it.
pub(in crate::ui::tui::zone::main::history) fn tool_node(
    block: &Block,
    paired: Option<&Block>,
    t: &HistoryTheme,
    o: &Opts<'_>,
    width: usize,
) -> Vec<Line<'static>> {
    let name = block.name.as_deref().unwrap_or_default();
    let args = block.args.as_deref().unwrap_or_default();
    let details = paired.and_then(details_of);
    let outcome = paired.map(|result| outcome_of(result, details.as_ref()));
    let mut out = Vec::new();

    if block.first
        && let Some(text) = block.text.as_deref()
        && !text.trim().is_empty()
    {
        out.extend(paint(text, t, width));
        out.push(block_gap());
    }

    if name == "todo" {
        out.extend(todo_marker(outcome, t));
        return out;
    }
    if name == "read" || name == "cd" {
        out.extend(match name {
            "read" => tools::read_line(args, outcome, o.cwd, t),
            _ => tools::cd_line(args, outcome, o.cwd, t),
        });
        return out;
    }
    out.extend(tool_card(
        Card {
            name,
            args,
            result: outcome,
            expanded: o.tool,
            tail: block.ok.is_none(),
            budget: o.fold,
        },
        t,
        width,
    ));
    out
}

/// The whole live tail, in the order upstream produced it.
pub(in crate::ui::tui::zone::main::history) fn live_rows(
    blocks: &[Block],
    t: &HistoryTheme,
    o: &Opts<'_>,
    width: usize,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    for (at, block) in blocks.iter().enumerate() {
        if block.kind == Kind::ToolResult {
            continue;
        }
        let paired = blocks.get(at + 1).filter(|next| block.answers(next));
        let rows = match block.kind {
            Kind::ToolRequest => tool_node(block, paired, t, o, width),
            Kind::User | Kind::Wake | Kind::Assistant | Kind::Reasoning | Kind::ToolResult => {
                super::super::block(block, None, t, o, width)
            }
        };
        if rows.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(block_gap());
        }
        out.extend(rows);
    }
    out
}

/// The memo a tool result carries, decoded.
fn details_of(result: &Block) -> Option<Value> {
    result
        .memo
        .as_deref()
        .and_then(|memo| serde_json::from_str(memo).ok())
}

/// What the paired result says about the call.
fn outcome_of<'a>(result: &'a Block, details: Option<&'a Value>) -> ToolOutcome<'a> {
    ToolOutcome {
        text: result.result.as_deref().unwrap_or_default(),
        ok: result.ok.unwrap_or(false),
        details,
        duration_ms: result.duration_ms.max(0) as u64,
    }
}

fn todo_marker(outcome: Option<ToolOutcome<'_>>, t: &HistoryTheme) -> Vec<Line<'static>> {
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
