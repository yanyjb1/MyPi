//! `profile` — the prompt a session will run with, chosen before its first message.

use anyhow::Result;
use serde_json::Value;

use crate::server::agent::r#loop::Change;
use crate::server::agent::tools::registry;
use crate::server::api::route::{i64_of, str_of};
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let name = str_of(ask, "name")?;
    emit::require_writer(api, viewer, session)?;
    let reader = Reader::open(api.db())?;
    let used = !reader.list(session)?.is_empty();
    drop(reader);
    if used {
        return Err(Refuse::Permission(
            "a profile changes only before the first message".into(),
        ));
    }
    api.ensure_booked(session)?;
    let resolved = api.cfg().resolve_profile(Some(name))?;
    let tools = resolved.tools.unwrap_or_else(registry::names);
    api.ledger().change(
        session,
        Change::Profile {
            system: resolved.system,
            tools,
            compact: resolved.compact_instruction,
        },
    )?;
    Ok(serde_json::json!({}))
}
