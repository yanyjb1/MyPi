//! todo: one line per call; the checklist itself is pinned under the transcript.

use ratatui::style::Modifier;
use serde_json::Value;

use super::super::super::row::{Ink, Lead, Row, Run};
use super::{State, ToolOutcome, arg_bool, call_input, error_under, partial};
use crate::ui::tui::theme::fixed;

/// The one row an exchange leaves: its state, its operation, and what it left behind.
pub(crate) fn marker(args: &str, outcome: Option<ToolOutcome<'_>>) -> Vec<Row> {
    let state = State::of(outcome);
    let op = partial::text(args, "type").unwrap_or_default();
    let done = arg_bool(args, "done").unwrap_or(true);
    let mut runs = vec![
        Run::with(state.glyph(), Some(state.glyph_ink()), Modifier::empty()),
        Run::fixed(" Todo", fixed::TOOL_TITLE).bold(),
    ];
    if !op.is_empty() {
        let word = if op == "tick" && !done { "untick" } else { &op };
        runs.push(Run::fixed(format!(" {word}"), fixed::MUTED));
        if op == "tick" {
            marker_tick(&mut runs, args, outcome, done);
        }
    }
    if let Some(meta) = meta(&op, outcome) {
        runs.push(Run::fixed(format!(" {meta}"), fixed::DIM));
    }
    let mut out = vec![Row::of(Lead::None, runs)];
    out.extend(error_under(outcome));
    out
}

/// The numbers a tick names, then the text of each, crossed out when it was a tick.
fn marker_tick(runs: &mut Vec<Run>, args: &str, outcome: Option<ToolOutcome<'_>>, done: bool) {
    let ids = ids(args);
    if ids.is_empty() {
        return;
    }
    let list = ids
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    runs.push(Run::fixed(format!(" {list}"), fixed::MUTED));
    let Some(text) = tick_text(outcome, &ids) else {
        return;
    };
    let attrs = if done {
        Modifier::CROSSED_OUT
    } else {
        Modifier::empty()
    };
    runs.push(Run::fixed(": ", fixed::DIM));
    runs.push(Run::with(text, Some(Ink::Fixed(fixed::DIM)), attrs));
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

/// The item numbers the call names, in parameter order.
fn ids(args: &str) -> Vec<u64> {
    let Some(Value::Array(list)) = call_input(args).and_then(|value| value.get("ids").cloned())
    else {
        return Vec::new();
    };
    list.iter().filter_map(number).collect()
}

/// One item number: an integer, or its decimal spelling.
fn number(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

/// The text of every named position, in the order the call names them.
fn tick_text(outcome: Option<ToolOutcome<'_>>, ids: &[u64]) -> Option<String> {
    let items = outcome?.details?.get("items")?.as_array()?;
    let texts: Vec<&str> = ids
        .iter()
        .filter_map(|id| usize::try_from(*id).ok()?.checked_sub(1))
        .filter_map(|at| items.get(at)?.get("text")?.as_str())
        .collect();
    (!texts.is_empty()).then(|| texts.join("; "))
}
