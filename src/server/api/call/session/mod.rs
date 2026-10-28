//! Session queries, and the read helpers the rest of the domain shares.

use anyhow::Result;
use rusqlite::Connection;
use std::collections::HashSet;

use crate::ipc::contract::SessionReply;
use crate::server::api::call::ctx::Ctx;
use crate::server::storage::decode::{
    BLOCK_COLS, TURN_COLS, TURN_FROM, block_from_row, turn_from_row,
};
use crate::server::storage::{BORROWED, Block, TurnRow};

pub mod blocks;
pub mod init;
pub mod meta;
pub mod usage;

pub use crate::ipc::contract::{
    InitReply, SessionAsk, SessionState, TurnBlock, TurnShape, UsageLang, UsageReply,
};

pub(crate) fn answer(ctx: &Ctx, session: i64, ask: SessionAsk) -> Result<SessionReply> {
    match ask {
        SessionAsk::Init { limit } => Ok(SessionReply::Init(init::answer(ctx, session, limit)?)),
        SessionAsk::Blocks { ranges } => {
            Ok(SessionReply::Blocks(blocks::answer(ctx, session, ranges)?))
        }
        SessionAsk::Meta => Ok(SessionReply::Meta(meta::answer(ctx, session)?)),
        SessionAsk::Usage { lang } => Ok(SessionReply::Usage(usage::answer(ctx, session, lang)?)),
    }
}

/// The read start: the newest compaction round at or before the list tail.
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
    for [lo, hi] in &turn.blocks {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {BLOCK_COLS} FROM blocks
              WHERE block_id >= ?1 AND block_id <= ?2 ORDER BY block_id"
        ))?;
        let found = stmt.query_map(rusqlite::params![lo, hi], block_from_row)?;
        for b in found {
            out.push(b?);
        }
    }
    Ok(out)
}

/// A borrowed round is an index copy of someone else's blocks.
pub(crate) fn borrowed(turn: &TurnRow) -> bool {
    turn.stop_reason.as_deref() == Some(BORROWED)
}

/// The user's own words inside one block.
pub(crate) fn user_text(block: &Block) -> Option<String> {
    match block.kind {
        crate::server::storage::BlockKind::User => block.content.clone(),
        _ => None,
    }
}

/// Every block id the session's list references.
pub(crate) fn block_ids(conn: &Connection, session: i64) -> Result<HashSet<i64>> {
    let mut out = HashSet::new();
    for turn_id in list_ids(conn, session, 0)? {
        if let Some(turn) = turn(conn, turn_id)? {
            for block in blocks_of(conn, &turn)? {
                out.insert(block.id);
            }
        }
    }
    Ok(out)
}
