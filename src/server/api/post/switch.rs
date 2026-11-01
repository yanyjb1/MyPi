//! `switch` — this session's model.

use anyhow::Result;
use serde_json::Value;

use crate::server::agent::r#loop::Change;
use crate::server::api::route::i64_of;
use crate::server::api::{Api, Refuse};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let (provider, model) = emit::model_of(
        ask.get("model")
            .ok_or_else(|| Refuse::Bad("missing `model`".into()))?,
    )?;
    if api
        .cfg()
        .model_by_id(&format!("{provider}:{model}"))
        .is_none()
    {
        return Err(Refuse::Bad(format!("no such model: {provider}:{model}")));
    }
    emit::require_writer(api, viewer, session)?;
    api.ensure_booked(session)?;
    api.ledger()
        .change(session, Change::Model { provider, model })?;
    emit::changed(api, viewer, session, "switch")?;
    Ok(serde_json::json!({}))
}
