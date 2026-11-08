//! Round rows: the round's own columns.

use anyhow::Result;

use super::super::decode::{encode_blocks, parse_blocks};

/// A new round row.
pub(crate) fn insert(
    tx: &rusqlite::Transaction<'_>,
    stop_reason: Option<&str>,
    model: Option<&(String, String)>,
) -> Result<i64> {
    let (provider, id) = match model {
        Some((provider, id)) => (Some(provider.as_str()), Some(id.as_str())),
        None => (None, None),
    };
    tx.execute(
        "INSERT INTO turns (stop_reason, provider, model) VALUES (?1, ?2, ?3)",
        rusqlite::params![stop_reason, provider, id],
    )?;
    Ok(tx.last_insert_rowid())
}

/// `turns.stop_reason`.
pub(crate) fn settle(
    tx: &rusqlite::Transaction<'_>,
    turn: i64,
    stop_reason: Option<&str>,
) -> Result<()> {
    tx.execute(
        "UPDATE turns SET stop_reason = ?1 WHERE turn_id = ?2",
        rusqlite::params![stop_reason, turn],
    )?;
    Ok(())
}

/// `turns.blocks`: the intervals this round occupies.
pub(crate) fn ranges(tx: &rusqlite::Transaction<'_>, turn: i64) -> Result<Vec<[i64; 2]>> {
    let raw: String = tx.query_row("SELECT blocks FROM turns WHERE turn_id = ?1", [turn], |r| {
        r.get(0)
    })?;
    parse_blocks(&raw)
}

/// `turns.blocks`: the intervals this round occupies.
pub(crate) fn set_blocks(
    tx: &rusqlite::Transaction<'_>,
    turn: i64,
    ranges: &[[i64; 2]],
) -> Result<()> {
    tx.execute(
        "UPDATE turns SET blocks = ?1 WHERE turn_id = ?2",
        rusqlite::params![encode_blocks(ranges), turn],
    )?;
    Ok(())
}
