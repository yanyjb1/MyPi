// `interrupt` — stop the running turn. Any front end watching the session may stop it, writer or not.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::route::i64_of;
use crate::server::api::{Api, Refuse};

pub(crate) fn apply(api: &Api, ask: &Value) -> Result<(), Refuse> {
    let session = i64_of(ask, "session")?;
    api.ledger().interrupt(session);
    Ok(())
}
