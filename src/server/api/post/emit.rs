// What every write shares: the writer gate and the model pair.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};

// Only the viewer that opened the running turn may change the session while it runs.
pub(crate) fn require_writer(api: &Api, viewer: &str, session: i64) -> Result<(), Refuse> {
    if api.relay().writes(session, viewer) {
        Ok(())
    } else {
        Err(Refuse::new("writer_required"))
    }
}

// A write that needs the session in the loop before it can change it.
pub(crate) fn prepare(api: &Api, viewer: &str, session: i64) -> Result<(), Refuse> {
    require_writer(api, viewer, session)?;
    api.ensure_booked(session)
}

// A model pair from `{"provider", "model"}`.
pub(crate) fn model_of(value: &Value) -> Result<(String, String), Refuse> {
    let provider = value
        .get("provider")
        .and_then(Value::as_str)
        .ok_or_else(|| Refuse::new("payload_key_missing").args(json!({"key": "model.provider"})))?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| Refuse::new("payload_key_missing").args(json!({"key": "model.model"})))?;
    Ok((provider.to_string(), model.to_string()))
}
