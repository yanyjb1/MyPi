// `switch` — this session's model.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::r#loop::Change;
use crate::server::api::route::i64_of;
use crate::server::api::{Api, Refuse};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let (provider, model) = emit::model_of(ask.get("model").ok_or_else(|| {
        Refuse::new("payload_key_missing").args(serde_json::json!({"key": "model"}))
    })?)?;
    let resolved = api
        .cfg()
        .model_by_id(&format!("{provider}:{model}"))
        .ok_or_else(|| {
            Refuse::new("model_unknown")
                .args(serde_json::json!({"address": format!("{provider}:{model}")}))
        })?;
    emit::prepare(api, viewer, session)?;
    let address = format!("{provider}:{model}");
    api.ledger().change(
        session,
        Change::Model {
            provider,
            model,
            vision: resolved.entry.vision,
        },
    )?;
    Ok(json!({"model": address}))
}
