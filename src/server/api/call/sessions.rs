//! `sessions` — the resume list, newest first.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::call::store::SESSION_COLS;
use crate::server::api::convert;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

pub(crate) fn answer(api: &Api, under: Option<&str>) -> Result<Value, Refuse> {
    let conn = Reader::open(api.db())?;
    let out = build(&conn, under);
    Reader::close(conn);
    Ok(json!({"rows": out?}))
}

fn build(conn: &rusqlite::Connection, under: Option<&str>) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SESSION_COLS}
           FROM sessions s
          WHERE (?1 IS NULL
                 OR s.id IN (SELECT DISTINCT session_id FROM cwd_history WHERE cwd = ?1))
            AND s.id NOT IN (SELECT subsession_id FROM subsessions)
          ORDER BY s.id DESC"
    ))?;
    let rows = stmt.query_map(
        rusqlite::params![under],
        crate::server::storage::decode::session_meta,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(convert::session_meta(row?));
    }
    Ok(out)
}
