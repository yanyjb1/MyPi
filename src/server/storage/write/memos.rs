//! Memo rows: append-only revisions grouped by scope and name.

use anyhow::{Context, Result, ensure};
use rusqlite::params;

use super::super::shape::{MemoOp, MemoScope};
use super::shared::{in_ranges, session_blocks};

/// Apply one memo operation. The caller supplies the session and the block that carries the write;
/// a current `Add` / `Mod` falls back to that block when no explicit anchor is given.
pub(crate) fn apply(
    tx: &rusqlite::Transaction<'_>,
    session: i64,
    block_id: i64,
    op: MemoOp,
    scope: MemoScope,
    anchor: Option<i64>,
    name: &str,
    content: Option<&str>,
    ts: &str,
) -> Result<()> {
    let name = name.trim();
    ensure!(!name.is_empty(), "a memo name is non-empty");
    ensure!(
        !name.chars().any(char::is_whitespace),
        "a memo name has no spaces"
    );

    match scope {
        MemoScope::Global => {
            ensure!(anchor.is_none(), "a global memo carries no anchor");
            match op {
                MemoOp::Add => {
                    let content = content.context("memo add needs content")?;
                    let exists: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM memos WHERE anchor IS NULL AND name = ?1",
                        params![name],
                        |r| r.get(0),
                    )?;
                    ensure!(exists == 0, "memo {name} already exists");
                    tx.execute(
                        "INSERT INTO memos (anchor, name, content, ts) VALUES (NULL, ?1, ?2, ?3)",
                        params![name, content, ts],
                    )?;
                }
                MemoOp::Mod => {
                    let content = content.context("memo mod needs content")?;
                    let exists: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM memos WHERE anchor IS NULL AND name = ?1",
                        params![name],
                        |r| r.get(0),
                    )?;
                    ensure!(exists > 0, "memo {name} does not exist");
                    tx.execute(
                        "INSERT INTO memos (anchor, name, content, ts) VALUES (NULL, ?1, ?2, ?3)",
                        params![name, content, ts],
                    )?;
                }
                MemoOp::Del => {
                    let deleted = tx.execute(
                        "DELETE FROM memos WHERE anchor IS NULL AND name = ?1",
                        params![name],
                    )?;
                    ensure!(deleted > 0, "memo {name} does not exist");
                }
            }
        }
        MemoScope::Current => {
            let ranges = session_blocks(tx, session)?;
            let rows = visible(tx, &ranges, name)?;
            match op {
                MemoOp::Add => {
                    ensure!(rows.is_empty(), "memo {name} already exists");
                    let content = content.context("memo add needs content")?;
                    let anchor = anchor.unwrap_or(block_id);
                    ensure!(anchor > 0, "a current memo needs a block");
                    tx.execute(
                        "INSERT INTO memos (anchor, name, content, ts) VALUES (?1, ?2, ?3, ?4)",
                        params![anchor, name, content, ts],
                    )?;
                }
                MemoOp::Mod => {
                    ensure!(!rows.is_empty(), "memo {name} does not exist");
                    let content = content.context("memo mod needs content")?;
                    let anchor = anchor.unwrap_or(block_id);
                    ensure!(anchor > 0, "a current memo needs a block");
                    tx.execute(
                        "INSERT INTO memos (anchor, name, content, ts) VALUES (?1, ?2, ?3, ?4)",
                        params![anchor, name, content, ts],
                    )?;
                }
                MemoOp::Del => {
                    ensure!(!rows.is_empty(), "memo {name} does not exist");
                    for (id, _) in rows {
                        tx.execute("DELETE FROM memos WHERE id = ?1", params![id])?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// The rows with this name whose anchor belongs to the session.
fn visible(
    tx: &rusqlite::Transaction<'_>,
    ranges: &[[i64; 2]],
    name: &str,
) -> Result<Vec<(i64, i64)>> {
    let mut stmt =
        tx.prepare("SELECT id, anchor FROM memos WHERE anchor IS NOT NULL AND name = ?1")?;
    let rows = stmt.query_map(params![name], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (id, anchor) = row?;
        if in_ranges(ranges, anchor) {
            out.push((id, anchor));
        }
    }
    Ok(out)
}
