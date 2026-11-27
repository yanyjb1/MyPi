// `effort` — how hard this session's laps ask the model to think.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::r#loop::Change;
use crate::server::api::route::i64_of;
use crate::server::api::{Api, Refuse};
use crate::server::config::Effort;

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let raw = ask
        .get("effort")
        .and_then(Value::as_str)
        .ok_or_else(|| Refuse::new("payload_key_missing").args(json!({"key": "effort"})))?;
    let effort = Effort::parse(raw)
        .ok_or_else(|| Refuse::new("effort_unknown").args(json!({"effort": raw})))?;
    emit::prepare(api, viewer, session)?;
    // The rungs a model takes are its own declaration; a front end hears the refusal here.
    let declared = declared(api, session)?;
    if declared.is_empty() {
        return Err(Refuse::new("effort_unsupported"));
    }
    if effort != Effort::Auto && !declared.contains(&effort) {
        let rungs = declared
            .iter()
            .map(|rung| rung.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Refuse::new("effort_not_declared")
            .args(json!({"effort": effort.as_str(), "declared": rungs})));
    }
    api.ledger().change(session, Change::Effort(effort))?;
    Ok(json!({"effort": effort.as_str()}))
}

// The rungs this session's model declares; empty takes none.
fn declared(api: &Api, session: i64) -> Result<Vec<Effort>, Refuse> {
    let (provider, model) = api.ledger().model(session)?;
    Ok(api
        .cfg()
        .model_by_id(&format!("{provider}:{model}"))
        .map(|resolved| resolved.entry.efforts)
        .unwrap_or_default())
}
