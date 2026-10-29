//! `init` — the whole structure, plus the newest blocks.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::call::store;
use crate::server::api::convert;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

use super::todo;

const LATEST: usize = 64;

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let conn = Reader::open(api.db())?;
    let out = build(&conn, session);
    Reader::close(conn);
    out
}

fn build(conn: &rusqlite::Connection, session: i64) -> Result<Value, Refuse> {
    let from = store::read_start(conn, session)?.unwrap_or(0);
    let ids = store::list_ids(conn, session, from)?;

    let mut structure = Vec::new();
    let mut blocks = Vec::new();
    for turn_id in ids.iter().rev() {
        let Some(row) = store::turn(conn, *turn_id)? else {
            continue;
        };
        structure.push(json!({"turn": *turn_id, "ranges": row.blocks}));
        if store::borrowed(&row) || blocks.len() >= LATEST {
            continue;
        }
        let mut list = store::blocks_of(conn, &row)?;
        list.reverse();
        for block in list {
            if blocks.len() == LATEST {
                break;
            }
            blocks.push(convert::block(block));
        }
    }
    blocks.reverse();

    let phases = todo::list(conn, session)?;
    let todo = if phases.is_empty() {
        Value::Null
    } else {
        Value::Array(phases.into_iter().map(convert::todo_phase).collect())
    };

    Ok(json!({
        "structure": structure,
        "blocks": blocks,
        "todo": todo,
    }))
}
