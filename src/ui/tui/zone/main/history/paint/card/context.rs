//! context: the call history as one table per operation.

use serde_json::Value;

use super::super::super::layout::line::Row;
use super::super::table;
use super::{Section, ToolBlock, ToolOutcome, ToolRenderer, arg_scalar, arg_str, output_lines};

pub(crate) struct Context;

impl ToolRenderer for Context {
    fn title(&self, b: &ToolBlock<'_>) -> Option<String> {
        let op = arg_str(b.args, "op")?;
        Some(match op.as_str() {
            "show" => format!("show the content of {}", listing(b.args, "ids")),
            _ => format!(
                "query {} in the scope of {}, top {}, {}",
                listing(b.args, "anchor"),
                defaulted(b.args, "tool", "bash"),
                defaulted(b.args, "limit", "3"),
                defaulted(b.args, "direction", "after"),
            ),
        })
    }

    fn call_body(&self, _b: &ToolBlock<'_>) -> Vec<Row> {
        Vec::new()
    }

    fn result_sections(&self, b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        let lines = match o.details {
            Some(details) => table_of(b, details, o.text),
            None => output_lines(o.text),
        };
        vec![Section { label: None, lines }]
    }
}

fn table_of(b: &ToolBlock<'_>, details: &Value, raw: &str) -> Vec<Row> {
    let Some(rows) = details.as_array() else {
        return output_lines(raw);
    };
    let show = arg_str(b.args, "op").as_deref() == Some("show");
    let width = b.inner_width();
    if show {
        let body = rows
            .iter()
            .map(|row| {
                vec![
                    listing_of(row.get("id")),
                    render(row.get("request")),
                    render(row.get("result")),
                ]
            })
            .collect();
        return table::cells(
            vec![
                "id".to_string(),
                "request".to_string(),
                "result".to_string(),
            ],
            body,
            width,
        );
    }
    let body = rows
        .iter()
        .map(|row| {
            vec![
                listing_of(row.get("id")),
                listing_of(row.get("tool")),
                if row.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                    "ok".to_string()
                } else {
                    "failed".to_string()
                },
                listing_of(row.get("summary")),
            ]
        })
        .collect();
    table::cells(
        vec![
            "block".to_string(),
            "tool".to_string(),
            "status".to_string(),
            "summary".to_string(),
        ],
        body,
        width,
    )
}

/// One argument as it reads in a title: strings bare, arrays joined.
fn listing(args: &str, key: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(args) else {
        return String::new();
    };
    listing_of(value.get(key))
}

/// One argument, or the value the tool uses when it is absent.
fn defaulted(args: &str, key: &str, fallback: &str) -> String {
    arg_scalar(args, key).unwrap_or_else(|| fallback.to_string())
}

fn listing_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| listing_of(Some(item)))
            .collect::<Vec<_>>()
            .join(", "),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// One JSON cell as one line.
fn render(value: Option<&Value>) -> String {
    match value {
        Some(Value::Null) | None => "(none)".to_string(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}
