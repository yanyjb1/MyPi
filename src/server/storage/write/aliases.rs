//! Alias rows: one row per command, anchored on the block that wrote it.

use anyhow::Result;

/// One alias row.
pub(crate) fn insert(
    tx: &rusqlite::Transaction<'_>,
    anchor: i64,
    name: &str,
    command: &str,
    ts: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO aliases (anchor, name, command, ts) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![anchor, name, command, ts],
    )?;
    Ok(())
}
