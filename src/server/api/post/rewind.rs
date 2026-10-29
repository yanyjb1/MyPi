//! `rewind` — cut the list back to a block.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::convert;
use crate::server::api::route::i64_of;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Target;

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let turn = i64_of(ask, "turn")?;
    let block = i64_of(ask, "block")?;
    emit::require_writer(api, viewer, session)?;
    if api.relay().running(session) || api.register().running(session) {
        return Err(Refuse::Busy("the session is busy".into()));
    }
    let cut = convert::cut(api.writer().rewind(Target {
        session,
        turn,
        block,
    })?);
    api.relay().post(
        session,
        "rewind",
        viewer,
        json!({"turn": turn, "block": block, "cut": cut}),
    );
    Ok(json!({"cut": cut}))
}
