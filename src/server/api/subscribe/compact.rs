//! `compact` — open a compaction turn and take the session's writer slot.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::r#loop::register::{Message, Retention};
use crate::server::api::relay::Role;
use crate::server::api::route::{i64_of, opt_str_of};
use crate::server::api::{Api, Refuse};

pub(crate) fn apply(api: &Api, conn: u64, _viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let retention = retention_of(ask)?;
    let focus = opt_str_of(ask, "focus")?.map(str::to_string);

    if !api.relay().claim(session) {
        return Err(Refuse::Busy("the session is already in a turn".into()));
    }
    let turn: i64 = match api
        .register()
        .deliver(session, Message::Compact { retention, focus })
    {
        Ok(Some(turn)) => turn,
        Ok(None) => {
            api.relay().release(session);
            return Err(Refuse::Internal("the loop opened no round".into()));
        }
        Err(e) => {
            api.relay().release(session);
            return Err(e.into());
        }
    };
    api.relay().declare(conn, session, Role::Writer, Some(turn));
    Ok(json!({"turn": turn}))
}

fn retention_of(ask: &Value) -> Result<Option<Retention>, Refuse> {
    let Some(value) = ask.get("retention") else {
        return Ok(None);
    };
    match value {
        Value::String(text) if text == "Default" => Ok(None),
        Value::Object(map) => {
            if let Some(tokens) = map.get("Tokens").and_then(Value::as_u64) {
                return Ok(Some(Retention::Tokens(tokens as usize)));
            }
            if let Some(blocks) = map.get("Blocks").and_then(Value::as_u64) {
                return Ok(Some(Retention::Blocks(blocks as usize)));
            }
            Err(Refuse::Bad("bad `retention`".into()))
        }
        _ => Err(Refuse::Bad("bad `retention`".into())),
    }
}
