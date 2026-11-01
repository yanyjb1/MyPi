//! `compact` — open a compaction turn and take the session's writer slot.

use anyhow::Result;
use serde_json::Value;

use crate::server::agent::r#loop::{Message, Retention};
use crate::server::api::route::{i64_of, opt_str_of};
use crate::server::api::{Api, Refuse};

use super::submit;

pub(crate) fn apply(api: &Api, conn: u64, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let message = Message::Compact {
        retention: retention_of(ask)?,
        focus: opt_str_of(ask, "focus")?.map(str::to_string),
    };
    submit::open(api, conn, viewer, session, message, "compact")
}

/// `{"chars": n}` or `{"blocks": n}`; absent keeps the configured tail.
fn retention_of(ask: &Value) -> Result<Option<Retention>, Refuse> {
    let value = match ask.get("retention") {
        Some(value) => value,
        None => return Ok(None),
    };
    let object = match value.as_object() {
        Some(object) => object,
        None => return Err(Refuse::Bad("bad `retention`".into())),
    };
    if let Some(chars) = object.get("chars").and_then(Value::as_u64) {
        return Ok(Some(Retention::Chars(chars as usize)));
    }
    if let Some(blocks) = object.get("blocks").and_then(Value::as_u64) {
        return Ok(Some(Retention::Blocks(blocks as usize)));
    }
    Err(Refuse::Bad("bad `retention`".into()))
}
