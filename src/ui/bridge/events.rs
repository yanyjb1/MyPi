// One adapter's view of a stream: the categories it asked for, at the length it accepts.

use serde_json::{Value, json};

use crate::ipc::contract::Frame;

use super::manifest::Adapter;

// The live kinds a round says, and the two the adapter may switch off.
const THINKING: &[&str] = &["thinking", "boundary"];
const TOOLS: &[&str] = &["arguments", "tool_start", "tool_progress", "tool_end"];

// One frame, as this adapter hears it: itself, nothing, or cut to its length.
pub fn take(adapter: &Adapter, frame: Frame) -> Vec<Frame> {
    let Some(kind) = frame.payload.get("kind").and_then(Value::as_str) else {
        return vec![frame];
    };
    if THINKING.contains(&kind) && !adapter.thinking {
        return Vec::new();
    }
    if TOOLS.contains(&kind) && !adapter.tools {
        return Vec::new();
    }
    if kind != "text" || adapter.chunk == 0 {
        return vec![frame];
    }
    let text = frame
        .payload
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    cut(&frame, text, adapter.chunk)
}

// A piece cut to the adapter's own length: one frame per length, the last carrying the remainder.
fn cut(frame: &Frame, text: &str, width: usize) -> Vec<Frame> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return vec![frame.clone()];
    }
    let mut out = Vec::new();
    for part in chars.chunks(width) {
        let mut piece = frame.clone();
        if let Some(object) = piece.payload.as_object_mut() {
            object.insert("text".to_string(), json!(part.iter().collect::<String>()));
        }
        out.push(piece);
    }
    out
}
