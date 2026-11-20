// `rewind` — cut the list back to a block.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::route::i64_of;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Target;

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let turn = i64_of(ask, "turn")?;
    let block = i64_of(ask, "block")?;
    emit::require_writer(api, viewer, session)?;
    if api.relay().busy(session) {
        return Err(Refuse::new("busy"));
    }
    api.writer().rewind(Target {
        session,
        turn,
        block,
    })?;
    api.ledger().forget(session);
    Ok(json!({"turn": turn, "block": block}))
}
