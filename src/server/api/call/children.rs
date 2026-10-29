//! `children` — the sessions a `task` call spawned.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::call::store;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let conn = Reader::open(api.db())?;
    let out = build(&conn, session);
    Reader::close(conn);
    Ok(out?)
}

fn build(conn: &rusqlite::Connection, session: i64) -> Result<Value> {
    let anchors = store::block_ids(conn, session)?;
    let mut stmt = conn.prepare("SELECT subsession_id, anchor FROM subsessions")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (child, anchor) = row?;
        if anchors.contains(&anchor) {
            out.push(json!({"session": child, "anchor": anchor}));
        }
    }
    Ok(json!({"rows": out}))
}
