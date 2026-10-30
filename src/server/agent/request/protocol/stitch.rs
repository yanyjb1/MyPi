//! Tool calls stitched out of stream deltas. Shared mechanics, no protocol switches.

use serde_json::Value;

use super::super::message::WireCall;

/// Calls accumulating across deltas, keyed by the index the wire uses.
#[derive(Default)]
pub struct Calls {
    pending: Vec<(u64, WireCall)>,
}

impl Calls {
    /// Absorb one delta. Every argument fragment is handed to `emit` with its call id.
    pub fn feed(&mut self, delta: &Value, emit: &mut dyn FnMut(&str, &str)) {
        let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) else {
            return;
        };
        for raw in calls {
            let index = raw.get("index").and_then(Value::as_u64).unwrap_or(0);
            let entry = match self.pending.iter_mut().find(|(i, _)| *i == index) {
                Some((_, call)) => call,
                None => {
                    self.pending.push((
                        index,
                        WireCall::new(String::new(), String::new(), String::new()),
                    ));
                    &mut self.pending.last_mut().expect("just pushed").1
                }
            };
            if let Some(id) = raw.get("id").and_then(Value::as_str)
                && !id.is_empty()
            {
                entry.id = id.to_string();
            }
            let Some(function) = raw.get("function") else {
                continue;
            };
            if let Some(name) = function.get("name").and_then(Value::as_str)
                && !name.is_empty()
            {
                entry.function.name = name.to_string();
            }
            if let Some(piece) = function.get("arguments").and_then(Value::as_str) {
                if piece.is_empty() {
                    continue;
                }
                entry.function.arguments.push_str(piece);
                let id = entry.id.clone();
                emit(&id, piece);
            }
        }
    }

    pub fn finish(self) -> Vec<WireCall> {
        let mut calls = self.pending;
        calls.sort_by_key(|(index, _)| *index);
        calls.into_iter().map(|(_, call)| call).collect()
    }
}
