//! `switch` — this session's model.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::route::i64_of;
use crate::server::api::{Api, Refuse};
use crate::server::storage::{SessionChange, Submit};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let model = emit::model_of(
        ask.get("model")
            .ok_or_else(|| Refuse::Bad("missing `model`".into()))?,
    )?;
    emit::require_writer(api, viewer, session)?;

    let addressed = format!("{}:{}", model.0, model.1);
    api.writer().push(Submit::Session {
        session,
        change: SessionChange::Model(Some(model)),
    })?;
    api.register().retarget(session, &addressed)?;
    emit::changed(api, viewer, session, "switch")?;
    Ok(serde_json::json!({}))
}
