//! mcp: a tree of servers and tools, and a card for one call.

use serde_json::Value;

use super::super::super::row::{Lead, Row, Run};
use super::tree;
use super::{
    Section, State, ToolBlock, ToolOutcome, ToolRenderer, arg_str, error_lines, head, output_lines,
};
use crate::ui::tui::theme::fixed;

/// The one-line form; only `list` has one.
pub(crate) fn inline(args: &str, result: Option<ToolOutcome<'_>>) -> Option<Vec<Row>> {
    if arg_str(args, "op").as_deref() != Some("list") {
        return None;
    }
    let state = State::of(result);
    let mut out = vec![Row::of(Lead::None, head(state, "MCP"))];
    out.extend(servers(result));
    out.extend(error_lines(result));
    Some(out)
}

pub(crate) struct Call;

impl ToolRenderer for Call {
    fn title(&self, b: &ToolBlock<'_>) -> Option<String> {
        Some(format!(
            "call {} {} {}",
            arg_str(b.args, "server").unwrap_or_default(),
            arg_str(b.args, "tool").unwrap_or_default(),
            arguments(b.args),
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

fn arguments(args: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(args) else {
        return String::new();
    };
    match value.get("args") {
        Some(Value::Null) | None => "{}".to_string(),
        Some(other) => other.to_string(),
    }
}

fn servers(result: Option<ToolOutcome<'_>>) -> Vec<Row> {
    let entries: Vec<Value> = result
        .and_then(|o| o.details)
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default();
    let total = entries.len();
    let mut out = Vec::new();
    for (at, entry) in entries.iter().enumerate() {
        let last = at + 1 == total;
        let name = entry
            .get("server")
            .and_then(Value::as_str)
            .unwrap_or_default();
        out.push(tree::entry(last, vec![Run::plain(name)]));
        if let Some(error) = entry.get("error").and_then(Value::as_str) {
            out.push(tree::child(
                last,
                vec![Run::fixed(format!("(error: {error})"), fixed::ERR)],
            ));
            continue;
        }
        let tools = entry
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for tool in &tools {
            let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut runs = vec![
                Run::plain(name),
                tree::value(" : "),
                Run::plain(description),
            ];
            let required = tool
                .get("required")
                .and_then(Value::as_array)
                .map(|names| {
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            if !required.is_empty() {
                runs.push(tree::value(format!(" [required: {required}]")));
            }
            out.push(tree::child(last, runs));
        }
    }
    out
}
