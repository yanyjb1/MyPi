//! `interrupt` — stop the running turn.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::route::i64_of;
use crate::server::api::{Api, Refuse};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    emit::require_writer(api, viewer, session)?;
    api.ledger().interrupt(session);
    Ok(serde_json::json!({}))
}
