//! `profile` — the prompt and tool roster the session runs.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::route::{i64_of, str_of};
use crate::server::api::{Api, Refuse};
use crate::server::storage::{SessionChange, Submit};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let profile = str_of(ask, "profile")?.to_string();
    emit::require_writer(api, viewer, session)?;
    api.writer().push(Submit::Session {
        session,
        change: SessionChange::Profile(profile.clone()),
    })?;
    api.register().set(session, "profile", &profile)?;
    emit::changed(api, viewer, session, "profile")?;
    Ok(serde_json::json!({}))
}
