//! The cleanup half: a rewind's cut (prune mode) and a session's deletion, both followed by the
//! same sweep. Request archives are never swept.

pub(crate) mod rewind;
pub(crate) mod session;

use anyhow::Result;

use super::decode::parse_blocks;

/// Sweep what nothing references any more: rounds outside every list, blocks
/// outside every round's index, and the artifacts bound to those blocks.
pub(crate) fn prune(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    tx.execute(
        "DELETE FROM turns WHERE turn_id NOT IN (SELECT turn_id FROM session_turns)",
        [],
    )?;
    // The block survival set: every live round's intervals expanded into a temp table with a
    // recursive CTE (one interval can span thousands of blocks).
    tx.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS alive_blocks (id INTEGER PRIMARY KEY);
         DELETE FROM alive_blocks;",
    )?;
    let ranges: Vec<[i64; 2]> = {
        let mut stmt = tx.prepare("SELECT blocks FROM turns")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for raw in rows {
            out.extend(parse_blocks(&raw?).unwrap_or_default());
        }
        out
    };
    for [lo, hi] in ranges {
        tx.execute(
            "WITH RECURSIVE r(x) AS (SELECT ?1 UNION ALL SELECT x + 1 FROM r WHERE x < ?2)
             INSERT OR IGNORE INTO alive_blocks (id) SELECT x FROM r",
            rusqlite::params![lo, hi],
        )?;
    }
    tx.execute(
        "DELETE FROM blocks WHERE block_id NOT IN (SELECT id FROM alive_blocks)",
        [],
    )?;
    tx.execute("DROP TABLE alive_blocks", [])?;
    // An artifact follows its block: the block is gone, so the spill goes.
    tx.execute(
        "DELETE FROM artifacts WHERE block_id NOT IN (SELECT block_id FROM blocks)",
        [],
    )?;
    Ok(())
}
