//! `name` — the session's display name.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::route::{i64_of, opt_str_of};
use crate::server::api::{Api, Refuse};
use crate::server::storage::{SessionChange, Submit};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let name = opt_str_of(ask, "name")?.map(str::to_string);
    emit::require_writer(api, viewer, session)?;
    api.writer().push(Submit::Session {
        session,
        change: SessionChange::Name(name),
    })?;
    emit::changed(api, viewer, session, "name")?;
    Ok(serde_json::json!({}))
}
