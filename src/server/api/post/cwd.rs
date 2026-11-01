//! `cwd` — the working directory, handed to the loop.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::Value;

use crate::server::agent::r#loop::Change;
use crate::server::api::route::{i64_of, str_of};
use crate::server::api::{Api, Refuse};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let path = str_of(ask, "path")?;
    emit::require_writer(api, viewer, session)?;
    api.ensure_booked(session)?;
    api.ledger()
        .change(session, Change::Cwd(PathBuf::from(path)))?;
    emit::changed(api, viewer, session, "cwd")?;
    Ok(serde_json::json!({}))
}
