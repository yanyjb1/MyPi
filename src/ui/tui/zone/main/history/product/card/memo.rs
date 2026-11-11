//! memo: a written memo is a document, so it reads as markdown; the lists stay tables.

use serde_json::Value;

use super::super::super::row::Row;
use super::super::table;
use super::frame::markdown_numbered;
use super::{Section, ToolBlock, ToolOutcome, ToolRenderer, arg_str, error_body, output_lines};

pub(crate) struct Memo;

impl ToolRenderer for Memo {
    fn title(&self, b: &ToolBlock<'_>) -> Option<String> {
        let op = arg_str(b.args, "type")?;
        let name = arg_str(b.args, "name").unwrap_or_default();
        Some(match op.as_str() {
            "show" | "history" => format!("{op} {name} {}", scope(b.args)),
            "list" => format!("list {}", scope(b.args)),
            _ => format!("{op} -> {name}"),
        })
    }

    fn call_body(&self, b: &ToolBlock<'_>) -> Vec<Row> {
        let op = arg_str(b.args, "type").unwrap_or_default();
        if !matches!(op.as_str(), "add" | "mod") {
            return Vec::new();
        }
        arg_str(b.args, "content")
            .map(|content| markdown_numbered(&content, b.inner_width()))
            .unwrap_or_default()
    }

    fn result_sections(&self, b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        if o.ok == Some(false) {
            return vec![Section {
                label: None,
                lines: error_body(o.text),
            }];
        }
        let op = arg_str(b.args, "type").unwrap_or_default();
        let lines = match op.as_str() {
            "show" => shown(o.details, b.inner_width(), o.text),
            "history" | "list" => table_of(&op, o.details, b.inner_width(), o.text),
            _ => Vec::new(),
        };
        vec![Section { label: None, lines }]
    }
}

fn scope(args: &str) -> String {
    arg_str(args, "scope").unwrap_or_else(|| "current".to_string())
}

/// One memo read back: markdown, laid the way the conversation lays it.
fn shown(details: Option<&Value>, width: usize, raw: &str) -> Vec<Row> {
    match details
        .and_then(|d| d.get("content"))
        .and_then(Value::as_str)
    {
        Some(content) => markdown_numbered(content, width),
        None => output_lines(raw),
    }
}

fn table_of(op: &str, details: Option<&Value>, width: usize, raw: &str) -> Vec<Row> {
    let Some(details) = details else {
        return output_lines(raw);
    };
    match op {
        "history" => {
            let body = details
                .as_array()
                .map(|versions| {
                    versions
                        .iter()
                        .map(|version| {
                            vec![
                                version
                                    .get("ts")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                                version
                                    .get("content")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                            ]
                        })
                        .collect()
                })
                .unwrap_or_default();
            table::cells(vec!["time".to_string(), "content".to_string()], body, width)
        }
        _ => {
            let body = details
                .as_array()
                .map(|names| {
                    names
                        .iter()
                        .map(|name| vec![name.as_str().unwrap_or_default().to_string()])
                        .collect()
                })
                .unwrap_or_default();
            table::cells(vec!["name".to_string()], body, width)
        }
    }
}
