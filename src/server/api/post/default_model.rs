//! `default_model` — the model a new session opens with.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};

use super::emit;

pub(crate) fn apply(api: &Api, ask: &Value) -> Result<Value, Refuse> {
    let (provider, model) = emit::model_of(
        ask.get("model")
            .ok_or_else(|| Refuse::Bad("missing `model`".into()))?,
    )?;
    let path = api
        .cfg()
        .set_default_model(&format!("{provider}:{model}"))?;
    Ok(json!({"path": path.to_string_lossy()}))
}
