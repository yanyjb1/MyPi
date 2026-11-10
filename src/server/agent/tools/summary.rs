//! Reading a recorded call's arguments, and clipping a summary to one line.

use crate::server::agent::tools::contract::view::CallRecord;

/// One string field of a call's `input`.
pub(crate) fn field(call: &CallRecord, name: &str) -> Option<String> {
    input(&call.args)?.get(name)?.as_str().map(str::to_owned)
}

/// A call's arguments as a JSON object, the envelope taken off.
pub(crate) fn input(args: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(args)
        .ok()?
        .get("input")
        .cloned()
}

/// Whitespace runs collapsed to one space; past `cap` characters, cut and marked.
pub(crate) fn clip(text: &str, cap: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(cap) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat,
    }
}
