// `default_model` — the model a new session opens with.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::{Api, Refuse};

use super::emit;

pub(crate) fn apply(api: &Api, ask: &Value) -> Result<(), Refuse> {
    let (provider, model) = emit::model_of(ask.get("model").ok_or_else(|| {
        Refuse::new("payload_key_missing").args(serde_json::json!({"key": "model"}))
    })?)?;
    api.cfg()
        .set_default_model(&format!("{provider}:{model}"))?;
    Ok(())
}
