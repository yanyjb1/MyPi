//! Compaction's write: append the summary round, then one borrowed round per retained stretch.
//! A borrowed round copies an index, never content.

use anyhow::Result;

use super::super::decode::{BLOCK_BYTES_SQL, encode_blocks};
use super::super::shape::{NewBlock, RequestMeta};
use super::super::writer::Store;
use super::batches::land;
use super::shared::{append_list_row, put_request_meta, read_turn};

/// The stop reason a borrowed round carries: it is **not** a new round, only an index borrowed
/// from the original.
pub const BORROWED: &str = "borrowed";

impl Store {
    /// Append a compaction: the summary round, then the rounds that borrow their index from the
    /// retention region. `kept` says which round to borrow from and where its stretch starts.
    pub(crate) fn append_compaction(
        &mut self,
        session_id: i64,
        summary: &NewBlock,
        request: Option<&RequestMeta>,
        kept: &[(i64, Option<i64>)],
        profile: &str,
    ) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let request_id = match request {
            Some(r) => Some(put_request_meta(&tx, r)?),
            None => None,
        };
        let mut summary = summary.clone();
        summary.memo = Some(profile.to_string());
        // The summary round: one block, and its list row carries the flag
        // that makes it the read start.
        tx.execute(
            "INSERT INTO turns (request_id, stop_reason) VALUES (?1, 'compaction')",
            rusqlite::params![request_id],
        )?;
        let summary_turn = tx.last_insert_rowid();
        let (range, bytes, _) = land(&tx, session_id, std::slice::from_ref(&summary), &[])?;
        let range = range.expect("one block lands one interval");
        tx.execute(
            "UPDATE turns SET blocks = ?1, bytes = ?2 WHERE turn_id = ?3",
            rusqlite::params![encode_blocks(&[range]), bytes, summary_turn],
        )?;
        append_list_row(&tx, session_id, summary_turn, true)?;
        // The borrowed rounds: index copies of the retention region.
        for (turn_id, from) in kept {
            let source = read_turn(&tx, *turn_id)?
                .ok_or_else(|| anyhow::anyhow!("round {turn_id} is not in the retention region"))?;
            let (blocks, bytes) = match from {
                None => (source.blocks.clone(), source.bytes),
                Some(from) => {
                    let sliced = tail(&source.blocks, *from);
                    let bytes = payload_bytes(&tx, &sliced)?;
                    (sliced, bytes)
                }
            };
            tx.execute(
                "INSERT INTO turns (request_id, blocks, stop_reason, bytes)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![source.request_id, encode_blocks(&blocks), BORROWED, bytes],
            )?;
            let id = tx.last_insert_rowid();
            append_list_row(&tx, session_id, id, false)?;
        }
        tx.commit()?;
        Ok(summary_turn)
    }
}

/// The tail of a run of intervals, from block `from` on (a cut inside one
/// interval splits that interval).
fn tail(blocks: &[[i64; 2]], from: i64) -> Vec<[i64; 2]> {
    let mut out: Vec<[i64; 2]> = Vec::new();
    for [lo, hi] in blocks {
        if *hi < from {
            continue;
        }
        let lo = (*lo).max(from);
        if lo <= *hi {
            out.push([lo, *hi]);
        }
    }
    out
}

/// Total byte volume covered by a run of intervals (the same measure the
/// write path uses — `decode::BLOCK_BYTES_SQL`).
fn payload_bytes(tx: &rusqlite::Transaction<'_>, blocks: &[[i64; 2]]) -> Result<i64> {
    let mut total = 0i64;
    for [lo, hi] in blocks {
        total += tx.query_row(
            &format!(
                "SELECT COALESCE(SUM({BLOCK_BYTES_SQL}), 0) FROM blocks
                  WHERE block_id >= ?1 AND block_id <= ?2"
            ),
            rusqlite::params![lo, hi],
            |r| r.get::<_, i64>(0),
        )?;
    }
    Ok(total)
}
