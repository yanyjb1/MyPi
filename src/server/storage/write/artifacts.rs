//! The artifacts table: a block that spilled lands its artifact in the same transaction as the
//! block itself. The block's id is the key; reading one back is the caller's query.

use anyhow::Result;

/// Store one artifact's row, inside the transaction that lands its block.
pub(crate) fn put_artifact(
    tx: &rusqlite::Transaction<'_>,
    block: i64,
    name: &str,
    content: &str,
) -> Result<()> {
    let total_lines = content.lines().count() as i64;
    tx.execute(
        "INSERT INTO artifacts (block_id, name, total_lines, content)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![block, name, total_lines, content],
    )?;
    Ok(())
}
