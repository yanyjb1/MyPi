//! The child-session link: one row per spawned child, anchored on the block that spawned it.

use anyhow::Result;

/// One link row.
pub(crate) fn insert(
    tx: &rusqlite::Transaction<'_>,
    subsession_id: i64,
    anchor: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO subsessions (subsession_id, anchor) VALUES (?1, ?2)",
        rusqlite::params![subsession_id, anchor],
    )?;
    Ok(())
}
