//! kairos: one line per call, a tree for `list`, a card for `result`.

use serde_json::Value;

use super::super::super::row::{Lead, Row, Run};
use super::tree;
use super::{
    Section, State, ToolBlock, ToolOutcome, ToolRenderer, arg_str, error_under, head, item_error,
    output_lines, partial,
};
use crate::ui::tui::theme::fixed;

/// The one-line form; `result` has none.
pub(crate) fn inline(args: &str, result: Option<ToolOutcome<'_>>) -> Option<Vec<Row>> {
    let op = arg_str(args, "type")?;
    if op == "result" {
        return None;
    }
    let state = State::of(result);
    let label = |name: &str| head(state, name);
    let mut out = Vec::new();
    match op.as_str() {
        "now" | "after" | "every" => {
            let tasks = partial::items(args, "tasks");
            if tasks.is_empty() {
                out.push(Row::of(Lead::None, label("Kairos-Escrow")));
            }
            for (index, item) in tasks.iter().enumerate() {
                let content = partial::field(item, "content").unwrap_or_default();
                let line = if op == "now" {
                    format!(
                        "escrowed {content} (timeout: {})",
                        partial::field(item, "timeout").unwrap_or_default()
                    )
                } else {
                    format!("escrowed {content}, {op} {}", at(result, index))
                };
                let mut runs = head(State::of_item(result, index), "Kairos-Escrow");
                runs.push(Run::fixed(format!(": {line}"), fixed::MUTED));
                out.push(Row::of(Lead::None, runs));
                out.extend(item_error(result, index));
            }
        }
        "list" => {
            out.push(Row::of(Lead::None, label("Kairos-Inquiry")));
            out.extend(listing(result));
            out.extend(error_under(result));
        }
        "status" => {
            let mut runs = label("Kairos-Inquiry");
            runs.push(Run::fixed(format!(": {}", status(result)), fixed::MUTED));
            out.push(Row::of(Lead::None, runs));
            out.extend(error_under(result));
        }
        _ => {
            let id = arg_str(args, "id").unwrap_or_default();
            let mut runs = label("Kairos-Rescind");
            runs.push(Run::fixed(format!(": {id} canceled"), fixed::MUTED));
            out.push(Row::of(Lead::None, runs));
            out.extend(error_under(result));
        }
    }
    Some(out)
}

pub(crate) struct Payload;

impl ToolRenderer for Payload {
    fn title(&self, b: &ToolBlock<'_>) -> Option<String> {
        Some(format!(
            "result of {}",
            arg_str(b.args, "id").unwrap_or_default()
        ))
    }

    fn call_body(&self, _b: &ToolBlock<'_>) -> Vec<Row> {
        Vec::new()
    }

    fn result_sections(&self, _b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        vec![Section {
            label: None,
            lines: output_lines(o.text),
        }]
    }
}

fn at(result: Option<ToolOutcome<'_>>, index: usize) -> String {
    result
        .and_then(|o| o.details)
        .and_then(|d| d.get(index))
        .and_then(|d| d.get("at"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default()
}

fn status(result: Option<ToolOutcome<'_>>) -> String {
    let Some(details) = result.and_then(|o| o.details) else {
        return String::new();
    };
    let text = |key: &str| details.get(key).and_then(Value::as_str).map(str::to_string);
    let number = |key: &str| details.get(key).and_then(Value::as_i64);
    let mut out = format!(
        "{} => {}",
        text("id").unwrap_or_default(),
        text("state").unwrap_or_default()
    );
    if let Some(exit) = number("exit") {
        out.push_str(&format!(", exit {exit}"));
    }
    if let Some(duration) = number("duration").filter(|n| *n > 0) {
        out.push_str(&format!(", {duration}s"));
    }
    if let Some(bytes) = number("bytes").filter(|n| *n > 0) {
        out.push_str(&format!(", {bytes} bytes"));
    }
    if details
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        out.push_str(", truncated");
    }
    if let Some(next) = text("next") {
        out.push_str(&format!(", next {next}"));
    }
    if let Some(remaining) = number("remaining") {
        out.push_str(&format!(", remaining {remaining}"));
    }
    if let Some(skipped) = number("skipped").filter(|n| *n > 0) {
        out.push_str(&format!(", skipped {skipped}"));
    }
    out
}

fn listing(result: Option<ToolOutcome<'_>>) -> Vec<Row> {
    let rows: Vec<Value> = result
        .and_then(|o| o.details)
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default();
    let total = rows.len();
    rows.iter()
        .enumerate()
        .map(|(at, row)| {
            let text = |key: &str| {
                row.get(key)
                    .map(|value| match value {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default()
            };
            let runs = vec![
                Run::plain(text("id")),
                tree::value(" "),
                tree::value(text("kind")),
                tree::value(" "),
                tree::value(text("state")),
                tree::value(format!(" {}B", text("bytes"))),
            ];
            tree::entry(at + 1 == total, runs)
        })
        .collect()
}
