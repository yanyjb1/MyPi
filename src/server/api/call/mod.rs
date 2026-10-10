//! Call — one question, one answer. `answer` is the door: a read connection, the capability that
//! owns the question, the connection closed again.

use anyhow::Result;
use std::path::Path;

use rusqlite::Connection;

use crate::server::api::driver;
use crate::server::config::Config;
use crate::server::storage::decode::BLOCK_COLS;
use crate::server::storage::{Block, BlockKind, TurnRow};

use super::types::{Call, Reply};

pub mod complete;
pub mod meta;
pub mod sessions;
pub mod todo;
pub mod usage;
pub mod viewport;

/// Answer one call: open a read connection, answer it, close it again.
pub fn answer(cfg: &Config, call: Call) -> Result<Reply> {
    let state = driver::state(cfg)?;
    let state = state.lock().unwrap_or_else(|e| e.into_inner());
    state.call(call)
}

/// The dispatch, on a connection the caller owns.
pub(crate) fn dispatch(cfg: &Config, conn: &Connection, cwd: &Path, call: Call) -> Result<Reply> {
    match call {
        Call::Usage { session, lang } => Ok(Reply::Usage {
            lang,
            groups: usage::bill_for(cfg, conn, session)?,
        }),
        Call::Sessions { under } => Ok(Reply::SessionRows(sessions::rows(conn, under.as_deref())?)),
        Call::Tail { session, limit } => window(viewport::tail(conn, session, limit)?),
        Call::Older {
            session,
            before,
            limit,
        } => window(viewport::older(conn, session, before, limit)?),
        Call::Newer {
            session,
            after,
            limit,
        } => window(viewport::newer(conn, session, after, limit)?),
        Call::Todo { session } => Ok(Reply::Todo(todo::list(conn, session)?)),
        Call::Meta { session } => Ok(Reply::Meta(meta::state(conn, session, cwd)?)),
        Call::Pools => Ok(Reply::Pools(complete::pools(cfg))),
    }
}

/// A viewport answer: the blocks, and whether more exist past the window.
fn window((blocks, more): (Vec<Block>, bool)) -> Result<Reply> {
    Ok(Reply::Blocks { blocks, more })
}

/// The read start: the newest compaction round at or before the list tail (`None` = never
/// compacted).
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

/// One round's blocks, in interval order (= write order).
fn blocks_of(conn: &Connection, turn: &TurnRow) -> Result<Vec<Block>> {
    let mut out = Vec::new();
    for [lo, hi] in &turn.blocks {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {BLOCK_COLS} FROM blocks
              WHERE block_id >= ?1 AND block_id <= ?2 ORDER BY block_id"
        ))?;
        let found = stmt.query_map(
            rusqlite::params![lo, hi],
            crate::server::storage::decode::block_from_row,
        )?;
        for b in found {
            out.push(b?);
        }
    }
    Ok(out)
}

/// The user's own words inside one block (`None` = it carries none).
fn user_text(block: &Block) -> Option<String> {
    match block.kind {
        BlockKind::User => block.content.clone(),
        _ => None,
    }
}
