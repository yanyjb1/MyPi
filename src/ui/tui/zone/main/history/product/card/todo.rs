//! todo: the pinned marker, or the phases of the checklist.

use ratatui::style::Modifier;
use serde_json::Value;

use super::super::super::row::{Ink, Lead, Row, Run};
use super::{Section, ToolBlock, ToolOutcome, ToolRenderer, arg_str};
use crate::ui::tui::text::glyphs as g;
use crate::ui::tui::theme::fixed;

pub(crate) struct Todo;

impl ToolRenderer for Todo {
    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        let op = arg_str(b.args, "op").unwrap_or_default();
        (!op.is_empty()).then_some(op).into_iter().collect()
    }

    fn result_measure(&self, o: ToolOutcome<'_>) -> Option<String> {
        let d = o.details?;
        let (done, total) = (
            d.get("done").and_then(|v| v.as_u64())?,
            d.get("total").and_then(|v| v.as_u64())?,
        );
        Some(format!("{done}/{total} done"))
    }

    fn call_body(&self, b: &ToolBlock<'_>) -> Vec<Row> {
        if b.result.is_some() {
            return Vec::new();
        }
        arg_str(b.args, "task")
            .map(|task| {
                vec![Row::of(
                    Lead::None,
                    vec![Run::fixed(format!("→ {task}"), fixed::MUTED)],
                )]
            })
            .unwrap_or_default()
    }

    fn result_sections(&self, _b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        let Some(d) = o.details else {
            return Vec::new();
        };
        let Some(phases) = d.get("phases").and_then(|p| p.as_array()) else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        for phase in phases {
            let name = phase.get("name").and_then(|n| n.as_str()).unwrap_or("");
            lines.push(Row::of(
                Lead::None,
                vec![Run::fixed(name.to_string(), fixed::TOOL_TITLE)],
            ));
            for task in phase
                .get("tasks")
                .and_then(|t| t.as_array())
                .into_iter()
                .flatten()
            {
                lines.push(todo_line(task));
            }
        }
        if lines.is_empty() {
            return Vec::new();
        }
        vec![Section { label: None, lines }]
    }
}

/// The one row an exchange leaves in place of its card.
pub(crate) fn marker(outcome: Option<ToolOutcome<'_>>) -> Vec<Row> {
    let (glyph, ink) = match outcome {
        Some(o) if !o.ok => (g::ERR, Ink::Fixed(fixed::ERR)),
        _ => (g::PENDING, Ink::Accent),
    };
    let mut out = vec![Row::of(
        Lead::None,
        vec![
            Run::with(glyph, Some(ink), Modifier::empty()),
            Run::with(" Todo", Some(ink), Modifier::empty()),
        ],
    )];
    if let Some(o) = outcome.filter(|o| !o.ok) {
        for part in o.text.split('\n').filter(|s| !s.trim().is_empty()) {
            out.push(Row::of(
                Lead::None,
                vec![Run::fixed(format!("  {part}"), fixed::ERR)],
            ));
        }
    }
    out
}

fn todo_line(task: &Value) -> Row {
    let status = task
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("pending");
    let content = task.get("content").and_then(|c| c.as_str()).unwrap_or("");
    let (glyph, ink) = match status {
        "done" => (g::TODO_DONE, Ink::Fixed(fixed::OK)),
        "in_progress" => (g::TODO_ACTIVE, Ink::Accent),
        "blocked" => (g::TODO_BLOCKED, Ink::Fixed(fixed::WARN)),
        "abandoned" => (g::TODO_DROPPED, Ink::Fixed(fixed::DIM)),
        _ => (g::TODO_PENDING, Ink::Fixed(fixed::DIM)),
    };
    let body = if status == "done" {
        fixed::DIM
    } else {
        fixed::BODY
    };
    let mut runs = vec![
        Run::with(format!("{glyph} "), Some(ink), Modifier::empty()),
        Run::fixed(content.to_string(), body),
    ];
    if let Some(reason) = task.get("blocker").and_then(|b| b.as_str()) {
        runs.push(Run::fixed(format!("（卡住：{reason}）"), fixed::WARN));
    }
    Row::of(Lead::None, runs)
}
