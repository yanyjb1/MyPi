//! Alias rows: add one per block, remove the rows visible from the session.

use anyhow::{Context, Result};
use rusqlite::params;

use super::super::shape::AliasOp;
use super::shared::{in_ranges, session_blocks};

/// Apply one alias operation.
pub(crate) fn apply(
    tx: &rusqlite::Transaction<'_>,
    session: i64,
    block_id: i64,
    op: AliasOp,
    name: &str,
    command: Option<&str>,
    ts: &str,
) -> Result<()> {
    match op {
        AliasOp::Add => {
            let command = command.context("alias add needs a command")?;
            tx.execute(
                "INSERT INTO aliases (anchor, name, command, ts) VALUES (?1, ?2, ?3, ?4)",
                params![block_id, name, command, ts],
            )?;
        }
        AliasOp::Remove => {
            let ranges = session_blocks(tx, session)?;
            let mut stmt = tx.prepare("SELECT id, anchor FROM aliases WHERE name = ?1")?;
            let rows =
                stmt.query_map([name], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
            let mut ids = Vec::new();
            for row in rows {
                let (id, anchor) = row?;
                if in_ranges(&ranges, anchor) {
                    ids.push(id);
                }
            }
            drop(stmt);
            for id in ids {
                tx.execute("DELETE FROM aliases WHERE id = ?1", [id])?;
            }
        }
    }
    Ok(())
}
