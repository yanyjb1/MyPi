//! `name` — the session's display name.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::route::{i64_of, opt_str_of};
use crate::server::api::{Api, Refuse};
use crate::server::storage::{Row, Submit};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let name = opt_str_of(ask, "name")?.map(str::to_string);
    emit::require_writer(api, viewer, session)?;
    let changed = json!({"name": &name});
    api.writer().push(Submit {
        rows: vec![Row::Rename { session, name }],
    })?;
    emit::changed(api, viewer, session, changed);
    Ok(serde_json::json!({}))
}
