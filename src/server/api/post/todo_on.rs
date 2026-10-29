//! `todo_on` — the todo tool is switched on for this session.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::route::{i64_of, str_of};
use crate::server::api::{Api, Refuse};
use crate::server::storage::{SessionChange, Submit};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    switch(api, viewer, ask, true)
}

pub(crate) fn switch(api: &Api, viewer: &str, ask: &Value, on: bool) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let call_id = str_of(ask, "call_id")?.to_string();
    emit::require_writer(api, viewer, session)?;
    api.writer().push(Submit::Session {
        session,
        change: SessionChange::Todo { on, call_id },
    })?;
    emit::changed(
        api,
        viewer,
        session,
        if on { "todo_on" } else { "todo_off" },
    )?;
    Ok(serde_json::json!({}))
}
