//! alias: one line per call, and a tree for `list`.

use serde_json::Value;

use super::super::super::row::{Lead, Row, Run};
use super::tree;
use super::{State, ToolOutcome, arg_str, error_under, head, item_error, partial};
use crate::ui::tui::theme::fixed;

/// The whole exchange: one line per alias, or a tree of names.
pub(crate) fn block(args: &str, result: Option<ToolOutcome<'_>>) -> Vec<Row> {
    let state = State::of(result);
    let op = arg_str(args, "type").unwrap_or_default();
    let mut out = Vec::new();
    match op.as_str() {
        "add" => {
            let items = partial::items(args, "aliases");
            if items.is_empty() {
                out.push(Row::of(Lead::None, head(state, "Alias")));
            }
            for (index, item) in items.iter().enumerate() {
                let name = partial::field(item, "name").unwrap_or_default();
                let command = partial::field(item, "command").unwrap_or_default();
                let mut runs = head(State::of_item(result, index), "Alias");
                runs.push(Run::fixed(format!(": {name} => {command}"), fixed::MUTED));
                out.push(Row::of(Lead::None, runs));
                out.extend(item_error(result, index));
            }
        }
        "del" => {
            let names = partial::items(args, "names");
            if names.is_empty() {
                out.push(Row::of(Lead::None, head(state, "Alias")));
            }
            for (index, name) in names.iter().enumerate() {
                let mut runs = head(State::of_item(result, index), "Alias");
                runs.push(Run::fixed(
                    format!(": del {}", name.as_str().unwrap_or_default()),
                    fixed::MUTED,
                ));
                out.push(Row::of(Lead::None, runs));
                out.extend(item_error(result, index));
            }
        }
        _ => {
            out.push(Row::of(Lead::None, head(state, "Alias")));
            out.extend(list(result));
            out.extend(error_under(result));
        }
    }
    out
}

fn list(result: Option<ToolOutcome<'_>>) -> Vec<Row> {
    let entries: Vec<Value> = result
        .and_then(|o| o.details)
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default();
    let pairs: Vec<(String, String)> = if entries.is_empty() {
        result
            .map(|o| {
                o.text
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| (line.to_string(), String::new()))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        entries
            .iter()
            .map(|entry| {
                let name = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let command = entry
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                (name.to_string(), command.to_string())
            })
            .collect()
    };
    let total = pairs.len();
    pairs
        .into_iter()
        .enumerate()
        .map(|(at, (name, command))| {
            let last = at + 1 == total;
            let mut runs = vec![Run::plain(name)];
            if !command.is_empty() {
                runs.push(tree::value(" => "));
                runs.push(Run::plain(command));
            }
            tree::entry(last, runs)
        })
        .collect()
}
