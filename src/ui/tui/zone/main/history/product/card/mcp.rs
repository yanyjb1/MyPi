//! mcp: a tree of servers and tools, and a card for one call.

use serde_json::Value;

use super::super::super::row::{Lead, Row, Run};
use super::tree;
use super::{
    Section, State, ToolBlock, ToolOutcome, ToolRenderer, arg_str, error_under, head, item_error,
    loose, output_lines, partial,
};
use crate::ui::tui::theme::fixed;

/// The one-line form; only `list` has one.
pub(crate) fn inline(args: &str, result: Option<ToolOutcome<'_>>) -> Option<Vec<Row>> {
    if arg_str(args, "type").as_deref() != Some("list") {
        return None;
    }
    let state = State::of(result);
    let mut out = vec![Row::of(Lead::None, head(state, "MCP"))];
    out.extend(servers(result));
    out.extend(error_under(result));
    Some(out)
}

pub(crate) struct Call;

impl ToolRenderer for Call {
    fn title(&self, b: &ToolBlock<'_>) -> Option<String> {
        let items = partial::items(b.args, "calls");
        (items.len() == 1).then(|| target(&items[0]))
    }

    fn call_body(&self, _b: &ToolBlock<'_>) -> Vec<Row> {
        Vec::new()
    }

    fn result_sections(&self, b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        let calls = partial::items(b.args, "calls");
        if o.items.len() <= 1 {
            return vec![Section {
                label: None,
                lines: output_lines(o.text),
            }];
        }
        let bodies = bodies(o.text);
        let mut lines = Vec::new();
        for (index, item) in calls.iter().enumerate() {
            let mut runs = head(State::of_item(Some(o), index), "MCP");
            runs.push(Run::fixed(format!(": {}", target(item)), fixed::MUTED));
            lines.push(Row::of(Lead::None, runs));
            if o.items.get(index).is_some_and(|item| item.error.is_some()) {
                lines.extend(item_error(Some(o), index));
            } else if let Some(body) = bodies.get(index) {
                lines.extend(output_lines(&body.join("\n")));
            }
        }
        vec![Section {
            label: None,
            lines,
        }]
    }
}

/// One call item as its row names it.
fn target(item: &Value) -> String {
    let server = partial::field(item, "server").unwrap_or_default();
    let tool = partial::field(item, "tool").unwrap_or_default();
    let arguments = match item.get("args") {
        Some(Value::Null) | None => "{}".to_string(),
        Some(other) => loose(other).to_string(),
    };
    format!("call {server} {tool} {arguments}")
}

/// The text's item bodies: an `==> ` line starts the next one.
fn bodies(text: &str) -> Vec<Vec<&str>> {
    let mut out: Vec<Vec<&str>> = Vec::new();
    for line in text.lines() {
        if line.starts_with("==> ") {
            out.push(Vec::new());
        } else if out.is_empty() {
            out.push(vec![line]);
        } else if let Some(chunk) = out.last_mut() {
            chunk.push(line);
        }
    }
    out
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
