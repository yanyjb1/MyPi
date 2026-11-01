//! `watch` — this connection follows this session; nothing is asked, nothing is answered.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};

pub(crate) fn apply(api: &Api, conn: u64, session: i64) -> Result<Value, Refuse> {
    api.relay().declare(conn, session);
    Ok(json!({}))
}
