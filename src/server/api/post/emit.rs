//! What every write door shares: the writer gate, the event, and the model pair.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::{Api, Refuse};

/// Only the viewer that opened the running turn may change the session while it runs.
pub(crate) fn require_writer(api: &Api, viewer: &str, session: i64) -> Result<(), Refuse> {
    if api.relay().writes(session, viewer) {
        Ok(())
    } else {
        Err(Refuse::Permission(
            "only the session's writer may write".into(),
        ))
    }
}

/// Announce the field one write changed to everyone but the writer.
pub(crate) fn changed(api: &Api, viewer: &str, session: i64, changed: Value) {
    api.relay().change(session, changed, viewer);
}

/// A model pair from `{"provider", "model"}`.
pub(crate) fn model_of(value: &Value) -> Result<(String, String), Refuse> {
    let provider = value
        .get("provider")
        .and_then(Value::as_str)
        .ok_or_else(|| Refuse::Bad("missing or bad `model.provider`".into()))?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| Refuse::Bad("missing or bad `model.model`".into()))?;
    Ok((provider.to_string(), model.to_string()))
}
