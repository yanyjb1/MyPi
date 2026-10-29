//! `cwd` — the working directory, persisted on the session and handed to the loop.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::route::{i64_of, str_of};
use crate::server::api::{Api, Refuse};
use crate::server::storage::{SessionChange, Submit};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let path = str_of(ask, "path")?;
    emit::require_writer(api, viewer, session)?;

    let turn = emit::tip_turn(api, session)?;
    api.writer().push(Submit::Session {
        session,
        change: SessionChange::Cwd {
            turn,
            path: path.to_string(),
        },
    })?;
    api.register().set(session, "cwd", path)?;
    emit::changed(api, viewer, session, "cwd")?;
    Ok(serde_json::json!({}))
}
