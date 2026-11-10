//! memo: writes show their content, reads show a table.

use serde_json::Value;

use super::super::super::row::{Lead, Row, Run};
use super::super::table;
use super::{Section, ToolBlock, ToolOutcome, ToolRenderer, arg_str, error_body, output_lines};
use crate::ui::tui::theme::fixed;

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
            .map(|content| text_rows(&content))
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
            "show" | "history" | "list" => table_of(&op, o.details, b.inner_width(), o.text),
            _ => Vec::new(),
        };
        vec![Section { label: None, lines }]
    }
}

fn scope(args: &str) -> String {
    arg_str(args, "scope").unwrap_or_else(|| "current".to_string())
}

fn text_rows(text: &str) -> Vec<Row> {
    text.lines()
        .map(|line| Row::of(Lead::None, vec![Run::fixed(line, fixed::TOOL_OUTPUT)]))
        .collect()
}

fn table_of(op: &str, details: Option<&Value>, width: usize, raw: &str) -> Vec<Row> {
    let Some(details) = details else {
        return output_lines(raw);
    };
    match op {
        "show" => {
            let content = details
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default();
            table::cells(
                vec!["content".to_string()],
                vec![vec![content.to_string()]],
                width,
            )
        }
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
