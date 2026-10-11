//! The viewport: a bounded window of a conversation — the last N blocks, then older or newer across
//! round boundaries. Borrowed rows never enter a viewport.

use anyhow::Result;
use rusqlite::Connection;

use crate::server::storage::decode::{TURN_COLS, TURN_FROM};
use crate::server::storage::{Block, TurnRow};

use super::{blocks_of, read_start};

/// One block of a viewport answer, with the round it belongs to: a block can be named by two rounds
/// at once (its own, and a compaction's borrowed index), and a rewind has to pick one. `None` =
/// the round the caller named in the request: it is implied, so it is not sent back.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlacedBlock {
    pub turn: Option<i64>,
    pub block: Block,
}

/// The first frame: the last `limit` blocks plus whether older ones exist (counted in blocks).
pub fn tail(conn: &Connection, session: i64, limit: usize) -> Result<(Vec<PlacedBlock>, bool)> {
    let from = read_start(conn, session)?.unwrap_or(0);
    let mut collected: Vec<PlacedBlock> = Vec::new();
    let mut more = false;
    for turn_id in list_ids(conn, session, from)?.into_iter().rev() {
        let Some(turn) = turn(conn, turn_id)? else {
            continue;
        };
        if borrowed(&turn) {
            continue;
        }
        let mut blocks = blocks_of(conn, &turn)?;
        blocks.reverse();
        for b in blocks {
            if collected.len() == limit {
                more = true;
                break;
            }
            collected.push(PlacedBlock {
                turn: Some(turn_id),
                block: b,
            });
        }
        if more {
            break;
        }
    }
    collected.reverse();
    Ok((collected, more))
}

/// The `limit` blocks older than `before`, ascending; plus whether older
/// ones exist.
pub fn older(
    conn: &Connection,
    session: i64,
    before: i64,
    limit: usize,
    asked: Option<i64>,
) -> Result<(Vec<PlacedBlock>, bool)> {
    let placed = |turn_id: i64, block: Block| PlacedBlock {
        turn: (asked != Some(turn_id)).then_some(turn_id),
        block,
    };
    let mut collected: Vec<PlacedBlock> = Vec::new();
    let mut more = false;
    for turn_id in list_ids(conn, session, 0)?.into_iter().rev() {
        let Some(turn) = turn(conn, turn_id)? else {
            continue;
        };
        if borrowed(&turn) {
            continue;
        }
        if turn.blocks.first().is_some_and(|r| r[0] >= before) {
            continue; // the whole round sits after the boundary
        }
        let mut blocks = blocks_of(conn, &turn)?;
        blocks.retain(|b| b.id < before);
        blocks.reverse();
        for b in blocks {
            if collected.len() == limit {
                more = true;
                break;
            }
            collected.push(placed(turn_id, b));
        }
        if more {
            break;
        }
    }
    collected.reverse();
    Ok((collected, more))
}

/// The `limit` blocks newer than `after`, ascending; plus whether newer ones
/// exist.
pub fn newer(
    conn: &Connection,
    session: i64,
    after: i64,
    limit: usize,
    asked: Option<i64>,
) -> Result<(Vec<PlacedBlock>, bool)> {
    let placed = |turn_id: i64, block: Block| PlacedBlock {
        turn: (asked != Some(turn_id)).then_some(turn_id),
        block,
    };
    let mut collected: Vec<PlacedBlock> = Vec::new();
    let mut more = false;
    for turn_id in list_ids(conn, session, 0)? {
        let Some(turn) = turn(conn, turn_id)? else {
            continue;
        };
        if borrowed(&turn) {
            continue;
        }
        if turn.blocks.last().is_some_and(|r| r[1] <= after) {
            continue; // the whole round sits before the boundary
        }
        for b in blocks_of(conn, &turn)? {
            if b.id <= after {
                continue;
            }
            if collected.len() == limit {
                more = true;
                break;
            }
            collected.push(placed(turn_id, b));
        }
        if more {
            break;
        }
    }
    Ok((collected, more))
}

/// A borrowed round is an index copy of someone else's blocks: what was
/// shown once is not shown again.
fn borrowed(turn: &TurnRow) -> bool {
    turn.stop_reason.as_deref() == Some(crate::server::storage::BORROWED)
}

/// The list's round ids from `from` on, in order.
fn list_ids(conn: &Connection, session: i64, from: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT turn_id FROM session_turns
          WHERE session_id = ?1 AND idx >= ?2 ORDER BY idx",
    )?;
    let found = stmt.query_map(rusqlite::params![session, from], |r| r.get(0))?;
    found
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

/// One round row (request archive included).
fn turn(conn: &Connection, turn_id: i64) -> Result<Option<TurnRow>> {
    let mut stmt = conn
        .prepare(&format!("SELECT {TURN_COLS} {TURN_FROM} WHERE t.turn_id = ?1"))?;
    let mut found = stmt.query_map([turn_id], crate::server::storage::decode::turn_from_row)?;
    match found.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}
