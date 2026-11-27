// The door's own hosting frames: a session handed to the bridge to keep alive, and the same one given back.

use serde_json::{Value, json};

use crate::ipc::contract::Frame;
use crate::ui::bridge::manager::Hub;

use super::respond;

// `host/attach` keeps a session booked with no front end watching, `host/release` gives it back, and `host/list` names what is kept.
pub fn apply(hub: &Hub, frame: &Frame) -> Result<Value, (u16, Value)> {
    if frame.name == "list" {
        let sessions: Vec<Value> = hub
            .kept()
            .into_iter()
            .map(|(session, turn)| json!({"session": session, "turn": turn}))
            .collect();
        return Ok(json!({"sessions": sessions}));
    }
    let session = frame
        .payload
        .get("session")
        .and_then(Value::as_i64)
        .ok_or_else(|| bad("a hosting frame names `session`"))?;
    match frame.name.as_str() {
        "attach" => hub
            .host(session)
            .map(|()| json!({}))
            .map_err(|e| (502, respond::error("upstream_closed", &e))),
        "release" => {
            hub.release(session);
            Ok(json!({}))
        }
        other => Err(bad(&format!(
            "the door answers `attach`, `release` and `list`, not `{other}`"
        ))),
    }
}

fn bad(message: &str) -> (u16, Value) {
    (400, respond::error("bad_request", message))
}
