//! Artifact rows: the full text of one oversized tool result, keyed by its block.

use anyhow::Result;

/// One artifact row.
pub(crate) fn insert(
    tx: &rusqlite::Transaction<'_>,
    block: i64,
    name: &str,
    content: &str,
    hash: &str,
    reference: &str,
) -> Result<()> {
    let total_lines = content.lines().count() as i64;
    tx.execute(
        "INSERT INTO artifacts (block_id, name, total_lines, content, hash, reference)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![block, name, total_lines, content, hash, reference],
    )?;
    Ok(())
}
