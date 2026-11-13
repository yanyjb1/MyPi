//! Reading a recorded call's arguments, and clipping a summary to one line.

use serde_json::Value;

use crate::server::agent::tools::contract::view::CallRecord;

/// One string field of a call's arguments.
pub(crate) fn field(call: &CallRecord, name: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&call.args)
        .ok()?
        .get(name)?
        .as_str()
        .map(str::to_owned)
}

/// One array field's elements, in the order the call gave them.
pub(crate) fn array(call: &CallRecord, name: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(&call.args)
        .ok()
        .and_then(|args| args.get(name).and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

/// The `path` of every object in one array field.
pub(crate) fn paths(call: &CallRecord, name: &str) -> Vec<String> {
    array(call, name)
        .iter()
        .filter_map(|item| item.get("path")?.as_str().map(str::to_owned))
        .collect()
}

/// The first item's text, plus how many others the call carried.
pub(crate) fn counted(first: &str, total: usize) -> String {
    match total {
        0 | 1 => first.to_string(),
        n => format!("{first} (+{})", n - 1),
    }
}

/// Whitespace runs collapsed to one space; past `cap` characters, cut and marked.
pub(crate) fn clip(text: &str, cap: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(cap) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat,
    }
}
