//! Reading a recorded call's arguments, and clipping a summary to one line.

use crate::server::agent::tools::CallRecord;

/// One string field of a call's JSON arguments.
pub(crate) fn field(call: &CallRecord, name: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&call.args)
        .ok()?
        .get(name)?
        .as_str()
        .map(str::to_owned)
}

/// Whitespace runs collapsed to one space; past `cap` characters, cut and marked.
pub(crate) fn clip(text: &str, cap: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(cap) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat,
    }
}
