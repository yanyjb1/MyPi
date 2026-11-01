//! `todo` — the list replayed from the session's own calls.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::{Api, Refuse};

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let items = api.ledger().todo(session)?;
    serde_json::to_value(items).map_err(|e| Refuse::Internal(e.to_string()))
}
