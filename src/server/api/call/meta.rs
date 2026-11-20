// `meta` — the session row, every column of it.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let reader = Reader::open(api.db())?;
    reader
        .session(session)?
        .ok_or_else(|| Refuse::new("no_session").args(serde_json::json!({"session": session})))
}
