//! `blocks` — closed id ranges, answered in request order.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::call::store;
use crate::server::api::convert;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

pub(crate) fn answer(api: &Api, session: i64, ranges: Vec<[i64; 2]>) -> Result<Value, Refuse> {
    let conn = Reader::open(api.db())?;
    let out = build(&conn, session, &ranges);
    Reader::close(conn);
    out
}

fn build(conn: &rusqlite::Connection, session: i64, ranges: &[[i64; 2]]) -> Result<Value, Refuse> {
    let mut wanted = Vec::new();
    for [lo, hi] in ranges {
        if lo > hi {
            return Err(Refuse::Bad(format!("reversed range: [{lo}, {hi}]")));
        }
        wanted.extend(*lo..=*hi);
    }

    let bound: std::collections::HashSet<i64> = store::block_ids(conn, session)?;
    let mut found = Vec::new();
    for id in wanted {
        if !bound.contains(&id) {
            return Err(Refuse::Bad(format!(
                "block {id} is not in session {session}"
            )));
        }
        let block = conn.query_row(
            &format!(
                "SELECT {} FROM blocks WHERE block_id = ?1",
                crate::server::storage::decode::BLOCK_COLS
            ),
            [id],
            crate::server::storage::decode::block_from_row,
        )?;
        found.push(convert::block(block));
    }
    Ok(json!({"blocks": found}))
}
