//! `mailbox` — the front end's one pending message: written whole, or taken back.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::r#loop::Letter;
use crate::server::api::route::{i64_of, opt_str_of};
use crate::server::api::{Api, Refuse};

use super::emit;

pub(crate) fn apply(api: &Api, conn: u64, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    emit::require_writer(api, viewer, session)?;
    let text = opt_str_of(ask, "text")?.unwrap_or_default();
    if text.trim().is_empty() {
        api.ledger().clear_mail(session)?;
        api.relay().drop_mail(session);
        return Ok(json!({"written": false}));
    }
    let letter = Letter {
        prefix: opt_str_of(ask, "prefix")?.map(str::to_string),
        text: text.to_string(),
        suffix: opt_str_of(ask, "suffix")?.map(str::to_string),
    };
    api.ensure_booked(session)?;
    api.ledger().write_mail(session, letter)?;
    api.relay().note_mail(conn, session, viewer);
    Ok(json!({"written": true}))
}
