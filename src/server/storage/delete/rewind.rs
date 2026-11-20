//! Rewind: cut the conversation back to one block. The surface names the round it was showing,
//! because a block can be referenced by two rounds at once.

use anyhow::{Context, Result};

use super::super::write::shared::{self, read_turn};
use super::super::write::turns;
use super::super::writer::Store;
use super::sweep;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub session: i64,
    pub turn: i64,
    pub block: i64,
}

impl Store {
    // The picked block stays; everything after it goes.
    pub fn rewind(&mut self, t: Target) -> Result<()> {
        let turn = read_turn(&self.conn, t.turn)?
            .with_context(|| format!("round {} does not exist", t.turn))?;
        let idx: i64 = self
            .conn
            .query_row(
                "SELECT idx FROM session_turns WHERE session_id = ?1 AND turn_id = ?2",
                rusqlite::params![t.session, t.turn],
                |r| r.get(0),
            )
            .with_context(|| format!("round {} is not in session {}'s list", t.turn, t.session))?;
        let last = turn
            .last_block()
            .with_context(|| format!("round {} has no blocks", t.turn))?;
        anyhow::ensure!(
            turn.blocks
                .iter()
                .any(|[lo, hi]| (*lo..=*hi).contains(&t.block)),
            "block {} is not in round {}",
            t.block,
            t.turn
        );

        let tx = self.conn.transaction()?;
        if t.block != last {
            turns::set_blocks(&tx, t.turn, &head(&turn.blocks, t.block))?;
        }
        shared::delete_list_after(&tx, t.session, idx)?;
        sweep::run(&tx)?;
        tx.commit()?;
        Ok(())
    }
}

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
