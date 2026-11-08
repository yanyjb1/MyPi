//! todo: one line per call; the checklist itself is pinned under the transcript.

use ratatui::style::Modifier;
use serde_json::Value;

use super::super::super::row::{Lead, Row, Run};
use super::{State, ToolOutcome, error_under, partial};
use crate::ui::tui::theme::fixed;

/// The one row an exchange leaves: its state, its operation, and what it left behind.
pub(crate) fn marker(args: &str, outcome: Option<ToolOutcome<'_>>) -> Vec<Row> {
    let state = State::of(outcome);
    let op = partial::text(args, "op").unwrap_or_default();
    let mut runs = vec![
        Run::with(state.glyph(), Some(state.glyph_ink()), Modifier::empty()),
        Run::fixed(" Todo", fixed::TOOL_TITLE).bold(),
    ];
    let mut detail = op.clone();
    if op == "tick"
        && let Some(ids) = ids(args)
    {
        detail.push(' ');
        detail.push_str(&ids);
    }
    if !detail.is_empty() {
        runs.push(Run::fixed(format!(" {detail}"), fixed::MUTED));
    }
    if let Some(meta) = meta(&op, outcome) {
        runs.push(Run::fixed(format!(" {meta}"), fixed::DIM));
    }
    let mut out = vec![Row::of(Lead::None, runs)];
    out.extend(error_under(outcome));
    out
}

/// What the call did to the list, from the result it produced.
fn meta(op: &str, outcome: Option<ToolOutcome<'_>>) -> Option<String> {
    let items = outcome?.details?.get("items")?.as_array()?;
    let total = items.len();
    let done = items
        .iter()
        .filter(|item| item.get("done").and_then(Value::as_bool) == Some(true))
        .count();
    Some(match op {
        "set" if total == 0 => "cleared".to_string(),
        "set" => format!("· {total} items"),
        _ => format!("· {done}/{total}"),
    })
}

fn ids(args: &str) -> Option<String> {
    let value: Value = serde_json::from_str(args).ok()?;
    let list = value.get("ids")?.as_array()?;
    let ids: Vec<String> = list
        .iter()
        .filter_map(|n| n.as_u64())
        .map(|n| n.to_string())
        .collect();
    (!ids.is_empty()).then(|| ids.join(","))
}
