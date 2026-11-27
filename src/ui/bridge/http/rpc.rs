// One request off the HTTP door: a frame for the server, or a refusal the adapter can read.

use serde_json::{Value, json};

use crate::ipc::contract::Frame;

use crate::ui::bridge::adapter::manifest::Adapter;

// The kinds a front end may send; `register` is the bridge's own word and never crosses.
const KINDS: &[&str] = &["call", "post", "subscribe", "host", "local"];

// What an adapter asked for, decided: the frame to carry, or the error to answer with.
pub fn frame_of(adapter: &Adapter, body: &Value) -> Result<Frame, (u16, Value)> {
    let kind = body
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("`kind` is missing"))?;
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("`name` is missing"))?;
    if !KINDS.contains(&kind) {
        return Err(bad(
            "`kind` is not one of call, post, subscribe, host, local",
        ));
    }
    if adapter.forbids(kind, name) {
        return Err((
            403,
            super::respond::error(
                "forbidden",
                &format!("{} may not send {kind}/{name}", adapter.name),
            ),
        ));
    }
    let payload = match body.get("payload") {
        Some(value) if value.is_object() => value.clone(),
        Some(_) | None => json!({}),
    };
    Ok(Frame::new(kind, name, payload))
}

// The session a frame names, when it names one.
pub fn session_of(frame: &Frame) -> Option<i64> {
    frame.payload.get("session").and_then(Value::as_i64)
}

// The answer as the door says it: the server's payload, or its error as one.
pub fn answer(frame: &Frame) -> Value {
    frame.payload.clone()
}

// Whether this frame starts following a conversation: the door answers it with a stream, not a reply.
pub fn is_watch(frame: &Frame) -> bool {
    frame.kind == "subscribe" && frame.name == "watch" && session_of(frame).is_some()
}

fn bad(message: &str) -> (u16, Value) {
    (400, super::respond::error("bad_request", message))
}
