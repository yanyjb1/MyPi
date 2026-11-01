//! `meta` — the session row, every column of it.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let reader = Reader::open(api.db())?;
    Ok(reader
        .session(session)?
        .ok_or_else(|| Refuse::Bad(format!("no session {session}")))?)
}
