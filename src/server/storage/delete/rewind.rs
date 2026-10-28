//! Rewind: cut the conversation back to one block (prune mode). The surface names the round it was
//! showing, because a block can be referenced by two rounds at once. See the folder's `README.md`.

use anyhow::{Context, Result};
use rusqlite::params;

use super::super::decode::BLOCK_BYTES_SQL;
use super::super::write::shared::read_turn;
use super::super::writer::Store;
use super::prune;

/// What the surface saw: which block the user picked, and which round was
/// on screen when they picked it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub session: i64,
    pub turn: i64,
    pub block: i64,
}

/// What survived the cut. The caller opens the next round on top of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    /// The cut fell on a round boundary: nothing had to be rescued.
    Boundary { turn: i64 },
    /// The cut fell inside a round: the kept head became a new round.
    Rescued { turn: i64 },
    /// The picked block was the round's opening user message: that round went with it, and the
    /// user retypes. `turn` = the round before it (`None` = the list is empty now).
    Retyped { turn: Option<i64> },
}

impl Store {
    /// Cut `session`'s list back to `block` of `turn`, then sweep what
    /// nothing references any more.
    pub fn rewind(&mut self, t: Target) -> Result<Cut> {
        let turn = read_turn(&self.conn, t.turn)?
            .with_context(|| format!("round {} does not exist", t.turn))?;
        let idx: i64 = self
            .conn
            .query_row(
                "SELECT idx FROM session_turns WHERE session_id = ?1 AND turn_id = ?2",
                params![t.session, t.turn],
                |r| r.get(0),
            )
            .with_context(|| format!("round {} is not in session {}'s list", t.turn, t.session))?;
        let first = turn
            .first_block()
            .with_context(|| format!("round {} has no blocks", t.turn))?;
        let last = turn
            .last_block()
            .with_context(|| format!("round {} has no blocks", t.turn))?;
        anyhow::ensure!(
            (first..=last).contains(&t.block),
            "block {} is not in round {}",
            t.block,
            t.turn
        );

        // The picked block is this round's opening user message = "I want
        // those words back": the whole round goes.
        let opens_with_user = t.block == first && opens_with_user(&self.conn, t.block)?;

        let tx = self.conn.transaction()?;
        let cut = if opens_with_user {
            tx.execute(
                "DELETE FROM session_turns WHERE session_id = ?1 AND idx >= ?2",
                params![t.session, idx],
            )?;
            tx.execute("DELETE FROM turns WHERE turn_id = ?1", [t.turn])?;
            Cut::Retyped {
                turn: previous_turn(&tx, t.session, idx)?,
            }
        } else if t.block == last {
            // Exactly the round's tail: the round survives whole, only what
            // follows it is cut away.
            tx.execute(
                "DELETE FROM session_turns WHERE session_id = ?1 AND idx > ?2",
                params![t.session, idx],
            )?;
            Cut::Boundary { turn: t.turn }
        } else {
            // Split in half: the kept head becomes a new round that copies the original's index.
            let kept = head(&turn.blocks, t.block);
            let bytes = block_bytes(&tx, &kept)?;
            tx.execute(
                "INSERT INTO turns (request_id, blocks, stop_reason, bytes)
                 VALUES (?1, ?2, 'rewind', ?3)",
                params![turn.request_id, serde_json::to_string(&kept)?, bytes],
            )?;
            let rescued = tx.last_insert_rowid();
            tx.execute(
                "UPDATE session_turns SET turn_id = ?1, is_compaction = 0
                  WHERE session_id = ?2 AND idx = ?3",
                params![rescued, t.session, idx],
            )?;
            tx.execute(
                "DELETE FROM session_turns WHERE session_id = ?1 AND idx > ?2",
                params![t.session, idx],
            )?;
            Cut::Rescued { turn: rescued }
        };
        prune(&tx)?;
        tx.commit()?;
        Ok(cut)
    }
}

/// The list item just before `idx` (`None` = there is none).
fn previous_turn(tx: &rusqlite::Transaction<'_>, session: i64, idx: i64) -> Result<Option<i64>> {
    Ok(tx
        .query_row(
            "SELECT turn_id FROM session_turns
              WHERE session_id = ?1 AND idx < ?2 ORDER BY idx DESC LIMIT 1",
            params![session, idx],
            |r| r.get(0),
        )
        .ok())
}

/// Is this block a user message? (The block carries its kind already.)
fn opens_with_user(conn: &rusqlite::Connection, block: i64) -> Result<bool> {
    let kind: String = conn.query_row(
        "SELECT type FROM blocks WHERE block_id = ?1",
        [block],
        |r| r.get(0),
    )?;
    Ok(kind == "user")
}

/// The head of a run of intervals, up to and including block `upto`.
fn head(blocks: &[[i64; 2]], upto: i64) -> Vec<[i64; 2]> {
    let mut out: Vec<[i64; 2]> = Vec::new();
    for [lo, hi] in blocks {
        if *lo > upto {
            break;
        }
        out.push([*lo, (*hi).min(upto)]);
    }
    out
}

/// Total byte volume covered by a run of intervals (the same measure the
/// write path uses — `decode::BLOCK_BYTES_SQL`).
fn block_bytes(tx: &rusqlite::Transaction<'_>, blocks: &[[i64; 2]]) -> Result<i64> {
    let mut total = 0i64;
    for [lo, hi] in blocks {
        total += tx.query_row(
            &format!(
                "SELECT COALESCE(SUM({BLOCK_BYTES_SQL}), 0) FROM blocks
                  WHERE block_id >= ?1 AND block_id <= ?2"
            ),
            params![lo, hi],
            |r| r.get::<_, i64>(0),
        )?;
    }
    Ok(total)
}
