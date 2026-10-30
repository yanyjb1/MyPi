//! The one sweep: everything a delete leaves behind, removed in the same transaction.

use anyhow::Result;

use super::super::decode::parse_blocks;

/// Sweep what nothing references any more: list places and trail entries of dead rows, rounds
/// outside every list, blocks outside every round's index, and the rows anchored to those blocks.
pub(crate) fn run(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    tx.execute(
        "DELETE FROM session_turns WHERE session_id NOT IN (SELECT id FROM sessions)",
        [],
    )?;
    tx.execute(
        "DELETE FROM session_turns WHERE turn_id NOT IN (SELECT turn_id FROM turns)",
        [],
    )?;
    tx.execute(
        "DELETE FROM cwd_history WHERE session_id NOT IN (SELECT id FROM sessions)",
        [],
    )?;
    tx.execute(
        "DELETE FROM cwd_history WHERE turn_id NOT IN (SELECT turn_id FROM turns)",
        [],
    )?;
    tx.execute(
        "DELETE FROM turns WHERE turn_id NOT IN (SELECT turn_id FROM session_turns)",
        [],
    )?;
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
    tx.execute(
        "DELETE FROM artifacts WHERE block_id NOT IN (SELECT block_id FROM blocks)",
        [],
    )?;
    tx.execute(
        "DELETE FROM memos WHERE anchor IS NOT NULL AND anchor NOT IN (SELECT block_id FROM blocks)",
        [],
    )?;
    tx.execute(
        "DELETE FROM aliases WHERE anchor NOT IN (SELECT block_id FROM blocks)",
        [],
    )?;
    tx.execute(
        "DELETE FROM session_turns WHERE session_id IN (
            SELECT id FROM sessions WHERE anchor IS NOT NULL
              AND anchor NOT IN (SELECT block_id FROM blocks))",
        [],
    )?;
    tx.execute(
        "DELETE FROM cwd_history WHERE session_id IN (
            SELECT id FROM sessions WHERE anchor IS NOT NULL
              AND anchor NOT IN (SELECT block_id FROM blocks))",
        [],
    )?;
    tx.execute(
        "DELETE FROM sessions WHERE anchor IS NOT NULL
            AND anchor NOT IN (SELECT block_id FROM blocks)",
        [],
    )?;
    tx.execute(
        "UPDATE sessions SET meta = NULL
          WHERE meta IS NOT NULL
            AND meta NOT IN (SELECT request_id FROM request_meta)",
        [],
    )?;
    tx.execute(
        "DELETE FROM request_meta
          WHERE request_id NOT IN (SELECT meta FROM sessions WHERE meta IS NOT NULL)",
        [],
    )?;
    Ok(())
}
