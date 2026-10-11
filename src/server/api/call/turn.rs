//! One round's structure: which blocks it manages, newest first, each with the hash that claims
//! its content. A round's index is interval pairs (one per landed batch).

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use crate::server::api::types::TurnBlock;
use crate::server::storage::decode::{BLOCK_COLS, block_from_row, parse_blocks};

/// The round's blocks, newest first. A round that is not this session's answers nothing.
pub fn blocks(conn: &Connection, session: i64, turn: i64) -> Result<Vec<TurnBlock>> {
    let mine: Option<i64> = conn
        .query_row(
            "SELECT turn_id FROM session_turns WHERE session_id = ?1 AND turn_id = ?2",
            rusqlite::params![session, turn],
            |r| r.get(0),
        )
        .optional()?;
    if mine.is_none() {
        return Ok(Vec::new());
    }
    let index: String =
        conn.query_row("SELECT blocks FROM turns WHERE turn_id = ?1", [turn], |r| {
            r.get(0)
        })?;
    let mut out = Vec::new();
    for [lo, hi] in parse_blocks(&index)?.into_iter().rev() {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {BLOCK_COLS} FROM blocks
              WHERE block_id >= ?1 AND block_id <= ?2 ORDER BY block_id DESC"
        ))?;
        for row in stmt.query_map(rusqlite::params![lo, hi], block_from_row)? {
            let block = row?;
            out.push(TurnBlock {
                id: block.id,
                hash: block.content_key().digest(),
            });
        }
    }
    Ok(out)
}
