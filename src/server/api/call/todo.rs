//! `todo` — the list replayed from the session's own `todo` calls.

use anyhow::Result;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;

use crate::server::agent::request::message::{FunctionCall, ToolCall};
use crate::server::agent::tools::{Env, TodoPhase, execute};
use crate::server::api::convert;
use crate::server::api::{Api, Refuse};
use crate::server::storage::decode::{BLOCK_COLS, block_from_row, parse_blocks};
use crate::server::storage::{BORROWED, Block, BlockKind, Reader};

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let conn = Reader::open(api.db())?;
    let out = list(&conn, session);
    Reader::close(conn);
    Ok(Value::Array(
        out?.into_iter().map(convert::todo_phase).collect(),
    ))
}

/// The list through the session's gate.
pub(crate) fn list(conn: &Connection, session: i64) -> Result<Vec<TodoPhase>> {
    let (on, cwd) = enabled(conn, session)?;
    if !on {
        return Ok(Vec::new());
    }
    replay(conn, session, cwd)
}

fn enabled(conn: &Connection, session: i64) -> Result<(bool, Option<String>)> {
    Ok(conn.query_row(
        "SELECT todo_on, cwd FROM sessions WHERE id = ?1",
        [session],
        |r| Ok((r.get::<_, i64>(0)? != 0, r.get::<_, Option<String>>(1)?)),
    )?)
}

fn replay(conn: &Connection, session: i64, cwd: Option<String>) -> Result<Vec<TodoPhase>> {
    let blocks = blocks(conn, session)?;
    let results = outcomes(&blocks);
    let mut env = Env::new(cwd.unwrap_or_default());
    for block in &blocks {
        let Some(call) = replayable(block, &results) else {
            continue;
        };
        let _ = execute(&mut env, &call, &mut |_| {});
    }
    Ok(env.inbox.todo)
}

fn blocks(conn: &Connection, session: i64) -> Result<Vec<Block>> {
    let mut list = conn.prepare(
        "SELECT t.blocks, t.stop_reason FROM session_turns s
           JOIN turns t ON t.turn_id = s.turn_id
          WHERE s.session_id = ?1 ORDER BY s.idx",
    )?;
    let rounds = list
        .query_map([session], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<(String, Option<String>)>>>()?;

    let mut fetch = conn.prepare(&format!(
        "SELECT {BLOCK_COLS} FROM blocks
          WHERE block_id >= ?1 AND block_id <= ?2 ORDER BY block_id"
    ))?;
    let mut out = Vec::new();
    for (raw, stop) in rounds {
        if stop.as_deref() == Some(BORROWED) {
            continue;
        }
        for [lo, hi] in parse_blocks(&raw)? {
            let rows = fetch.query_map(rusqlite::params![lo, hi], block_from_row)?;
            for row in rows {
                out.push(row?);
            }
        }
    }
    Ok(out)
}

fn outcomes(blocks: &[Block]) -> HashMap<String, bool> {
    let mut out = HashMap::new();
    for block in blocks {
        if block.kind == BlockKind::ToolResult
            && let Some(id) = &block.call_id
        {
            out.insert(id.clone(), block.ok.unwrap_or(false));
        }
    }
    out
}

fn replayable(block: &Block, results: &HashMap<String, bool>) -> Option<ToolCall> {
    if block.kind != BlockKind::ToolRequest || block.name.as_deref() != Some("todo") {
        return None;
    }
    let id = block.call_id.as_deref()?;
    if !results.get(id).copied().unwrap_or(false) {
        return None;
    }
    Some(ToolCall {
        id: id.to_string(),
        kind: "function".to_string(),
        function: FunctionCall {
            name: "todo".to_string(),
            arguments: block.args.clone()?,
        },
    })
}
