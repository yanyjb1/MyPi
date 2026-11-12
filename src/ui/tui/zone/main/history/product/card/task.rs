//! task: the template, the budget, and the variables it was handed.

use serde_json::Value;

use super::super::super::row::{Lead, Row, Run};
use super::tree;
use super::{
    State, ToolOutcome, arg_scalar, arg_str, call_args, error_under, head, loose, relativize,
};
use crate::ui::tui::theme::fixed;

/// The whole exchange: one line, then the cwd and the variables.
pub(crate) fn block(args: &str, result: Option<ToolOutcome<'_>>, cwd: &str) -> Vec<Row> {
    let state = State::of(result);
    let template = template(args, cwd);
    let budget = arg_scalar(args, "maxToolCalls").unwrap_or_default();
    let mut runs = head(state, "task");
    let mut text = format!(": template -> {template}");
    if !budget.is_empty() {
        text.push_str(&format!(", toolbudget {budget}"));
    }
    runs.push(Run::fixed(text, fixed::MUTED));
    let mut out = vec![Row::of(Lead::None, runs)];
    out.extend(children(args, cwd));
    out.extend(error_under(result));
    out
}

fn template(args: &str, cwd: &str) -> String {
    let raw = arg_str(args, "template").unwrap_or_default();
    if raw.contains('/') {
        relativize(cwd, &raw)
    } else {
        raw
    }
}

fn children(args: &str, cwd: &str) -> Vec<Row> {
    let landed = arg_str(args, "cwd").filter(|path| !path.trim().is_empty());
    let vars = variables(args);
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

fn variables(args: &str) -> Vec<(String, String)> {
    let Some(object) = call_args(args)
        .and_then(|args| args.get("vars").cloned())
        .and_then(|vars| match loose(&vars) {
            Value::Object(map) => Some(map),
            _ => None,
        })
    else {
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
