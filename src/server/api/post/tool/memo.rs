//! Memo writes: current memos carry a block, global memos carry nothing.

use anyhow::{Result, bail};
use rusqlite::Transaction;

use crate::ipc::contract::call::memo::MemoScope;
use crate::ipc::contract::post::{MemoWrite, MemoWriteOp};
use crate::server::api::call::session::block_ids;
use crate::server::api::driver::Driver;
use crate::server::storage::decode::parse_blocks;
use crate::server::storage::write::memos;
use crate::server::storage::{MemoOp, MemoScope as StoredScope, Reader};

pub(crate) fn apply(driver: &Driver, write: MemoWrite) -> Result<()> {
    let mut conn = Reader::open(&driver.db)?;
    let tx = conn.transaction()?;
    let (session, anchor, scope) = match write.scope {
        MemoScope::Current => {
            let anchor = write
                .anchor
                .ok_or_else(|| anyhow::anyhow!("a current memo needs an anchor block"))?;
            let session = session_of_anchor(&tx, anchor)?;
            let anchors = block_ids(&tx, session)?;
            if !anchors.contains(&anchor) {
                bail!("block {anchor} is not in session {session}");
            }
            (session, Some(anchor), StoredScope::Current)
        }
        MemoScope::Global => {
            if write.anchor.is_some() {
                bail!("a global memo carries no anchor");
            }
            (0, None, StoredScope::Global)
        }
    };
    let op = match write.op {
        MemoWriteOp::Add => MemoOp::Add,
        MemoWriteOp::Mod => MemoOp::Mod,
        MemoWriteOp::Del => MemoOp::Del,
    };
    memos::apply(
        &tx,
        session,
        anchor.unwrap_or(0),
        op,
        scope,
        anchor,
        &write.name,
        write.content.as_deref(),
        &crate::server::platform::time::now_stamp(),
    )?;
    tx.commit()?;
    Ok(())
}

fn session_of_anchor(tx: &Transaction, anchor: i64) -> Result<i64> {
    let mut stmt = tx.prepare(
        "SELECT s.session_id, t.blocks FROM session_turns s
           JOIN turns t ON t.turn_id = s.turn_id
          WHERE s.kind = 'own'",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (session, raw) = row?;
        for [lo, hi] in parse_blocks(&raw)? {
            if lo <= anchor && anchor <= hi {
                return Ok(session);
            }
        }
    }
    bail!("block {anchor} belongs to no session")
}
