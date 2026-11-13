//! task: the templates, their budgets, and the variables they were handed.

use serde_json::Value;

use super::super::super::row::{Lead, Row, Run};
use super::tree;
use super::{
    State, ToolOutcome, error_under, head, loose, partial, relativize,
};
use crate::ui::tui::theme::fixed;

/// The whole exchange: one line per child, each with its cwd and variables.
pub(crate) fn block(args: &str, result: Option<ToolOutcome<'_>>, cwd: &str) -> Vec<Row> {
    let state = State::of(result);
    let tasks = partial::items(args, "tasks");
    let mut out = Vec::new();
    if tasks.is_empty() {
        out.push(Row::of(Lead::None, head(state, "task")));
    }
    for (index, item) in tasks.iter().enumerate() {
        let template = partial::field(item, "template").unwrap_or_default();
        let template = if template.contains('/') {
            relativize(cwd, &template)
        } else {
            template
        };
        let mut text = format!(": template -> {template}");
        if let Some(budget) = scalar(item, "maxToolCalls") {
            text.push_str(&format!(", toolbudget {budget}"));
        }
        let mut runs = if index == 0 {
            head(state, "task")
        } else {
            vec![Run::fixed("      ", fixed::DIM)]
        };
        runs.push(Run::fixed(text, fixed::MUTED));
        out.push(Row::of(Lead::None, runs));
        out.extend(children(item, cwd));
    }
    out.extend(error_under(result));
    out
}

fn children(item: &Value, cwd: &str) -> Vec<Row> {
    let landed = partial::field(item, "cwd").filter(|path| !path.trim().is_empty());
    let vars = variables(item);
    let mut out = Vec::new();
    let last_branch = vars.is_empty();
    if let Some(path) = landed {
        out.push(tree::entry(
            last_branch,
            vec![Run::plain("cwd "), tree::value(relativize(cwd, &path))],
        ));
    }
    if !vars.is_empty() {
        out.push(tree::entry(true, vec![Run::plain("vars")]));
        for (key, value) in vars {
            out.push(tree::child(
                true,
                vec![Run::plain(format!("{key}={value}"))],
            ));
        }
    }
    out
}

fn variables(item: &Value) -> Vec<(String, String)> {
    let Some(object) = item.get("vars").and_then(|vars| match loose(vars) {
        Value::Object(map) => Some(map),
        _ => None,
    }) else {
        return Vec::new();
    };
    object
        .iter()
        .map(|(key, value)| {
            let text = match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            (key.clone(), text)
        })
        .collect()
}

fn scalar(item: &Value, key: &str) -> Option<String> {
    Some(match item.get(key)? {
        Value::Null => return None,
        Value::String(text) => text.clone(),
        other => other.to_string(),
    })
}
