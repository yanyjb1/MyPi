//! `meta` — the session's row, every column of it.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::call::store;
use crate::server::api::convert;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let conn = Reader::open(api.db())?;
    let row = store::session_row(&conn, session);
    Reader::close(conn);
    Ok(convert::session_meta(row?))
}
