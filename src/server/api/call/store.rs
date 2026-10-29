//! The reads the call doors share: the list, its rounds, and the blocks they own.

use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

use crate::server::storage::decode::{
    BLOCK_COLS, TURN_COLS, TURN_FROM, block_from_row, session_meta, turn_from_row,
};
use crate::server::storage::{BORROWED, Block, SessionMeta, TurnRow};

/// The session's columns, in `session_meta` order.
pub(crate) const SESSION_COLS: &str =
    "id, name, started_at, cwd, model_provider, model, profile, todo_on";

/// The newest compaction round at or before the list tail.
pub(crate) fn read_start(conn: &Connection, session: i64) -> Result<Option<i64>> {
    let tail: Option<i64> = conn.query_row(
        "SELECT MAX(idx) FROM session_turns WHERE session_id = ?1",
        [session],
        |r| r.get(0),
    )?;
    let Some(tail) = tail else {
        return Ok(None);
    };
    Ok(conn.query_row(
        "SELECT MAX(idx) FROM session_turns
          WHERE session_id = ?1 AND is_compaction AND idx <= ?2",
        rusqlite::params![session, tail],
        |r| r.get(0),
    )?)
}

/// The list's round ids from `from` on, in order.
pub(crate) fn list_ids(conn: &Connection, session: i64, from: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT turn_id FROM session_turns
          WHERE session_id = ?1 AND idx >= ?2 ORDER BY idx",
    )?;
    let found = stmt.query_map(rusqlite::params![session, from], |r| r.get(0))?;
    found
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

/// One round row, request archive included.
pub(crate) fn turn(conn: &Connection, turn_id: i64) -> Result<Option<TurnRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TURN_COLS} {TURN_FROM} WHERE t.turn_id = ?1"
    ))?;
    let mut found = stmt.query_map([turn_id], turn_from_row)?;
    match found.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// One round's blocks, in interval order.
pub(crate) fn blocks_of(conn: &Connection, turn: &TurnRow) -> Result<Vec<Block>> {
    let mut out = Vec::new();
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {BLOCK_COLS} FROM blocks
          WHERE block_id >= ?1 AND block_id <= ?2 ORDER BY block_id"
    ))?;
    for [lo, hi] in &turn.blocks {
        let found = stmt.query_map(rusqlite::params![lo, hi], block_from_row)?;
        for block in found {
            out.push(block?);
        }
    }
    Ok(out)
}

/// A borrowed round is an index copy of someone else's blocks.
pub(crate) fn borrowed(turn: &TurnRow) -> bool {
    turn.stop_reason.as_deref() == Some(BORROWED)
}

/// Every block id the session's list references.
pub(crate) fn block_ids(conn: &Connection, session: i64) -> Result<HashSet<i64>> {
    let mut out = HashSet::new();
    for turn_id in list_ids(conn, session, 0)? {
        if let Some(row) = turn(conn, turn_id)? {
            for block in blocks_of(conn, &row)? {
                out.insert(block.id);
            }
        }
    }
    Ok(out)
}

/// One session row.
pub(crate) fn session_row(conn: &Connection, session: i64) -> Result<SessionMeta> {
    Ok(conn.query_row(
        &format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"),
        [session],
        session_meta,
    )?)
}
