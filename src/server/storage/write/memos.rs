//! Memo rows: one row per version, anchored on the block that wrote it.

use anyhow::Result;

/// One memo row; `anchor` `None` is the global scope.
pub(crate) fn insert(
    tx: &rusqlite::Transaction<'_>,
    anchor: Option<i64>,
    name: &str,
    content: &str,
    ts: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO memos (anchor, name, content, ts) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![anchor, name, content, ts],
    )?;
    Ok(())
}
