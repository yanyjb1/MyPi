//! `submit` — open a turn and take the session's writer slot.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::r#loop::register::Message;
use crate::server::api::relay::Role;
use crate::server::api::route::{i64_of, opt_str_of, str_of};
use crate::server::api::{Api, Refuse};

pub(crate) fn apply(api: &Api, conn: u64, _viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let prefix = opt_str_of(ask, "prefix")?.map(str::to_string);
    let text = str_of(ask, "text")?.to_string();
    let suffix = opt_str_of(ask, "suffix")?.map(str::to_string);

    if !api.relay().claim(session, api.register().running(session)) {
        return Err(Refuse::Busy("the session is already in a turn".into()));
    }
    let turn: i64 = match api.register().deliver(
        session,
        Message::Submit {
            prefix,
            text,
            suffix,
        },
    ) {
        Ok(turn) => turn,
        Err(e) => {
            api.relay().release(session);
            return Err(e.into());
        }
    };
    api.relay().declare(conn, session, Role::Writer, Some(turn));
    Ok(json!({"turn": turn}))
}
