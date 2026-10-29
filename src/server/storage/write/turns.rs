//! Round rows: the round's own columns and the usage it ends with.

use anyhow::Result;

use super::super::decode::encode_blocks;
use super::super::shape::RoundUsage;

/// A new round row.
pub(crate) fn insert(
    tx: &rusqlite::Transaction<'_>,
    request: Option<i64>,
    stop_reason: Option<&str>,
    usage: Option<RoundUsage>,
) -> Result<i64> {
    let (p, c, k, w) = usage_columns(usage);
    tx.execute(
        "INSERT INTO turns (request_id, stop_reason, prompt_tokens, completion_tokens,
                cached_tokens, cache_write_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![request, stop_reason, p, c, k, w],
    )?;
    Ok(tx.last_insert_rowid())
}

/// `turns.stop_reason` + the four usage columns.
pub(crate) fn settle(
    tx: &rusqlite::Transaction<'_>,
    turn: i64,
    stop_reason: Option<&str>,
    usage: Option<RoundUsage>,
) -> Result<()> {
    let (p, c, k, w) = usage_columns(usage);
    tx.execute(
        "UPDATE turns SET stop_reason = ?1, prompt_tokens = ?2, completion_tokens = ?3,
                cached_tokens = ?4, cache_write_tokens = ?5
          WHERE turn_id = ?6",
        rusqlite::params![stop_reason, p, c, k, w, turn],
    )?;
    Ok(())
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

/// One usage, as its four columns: all null when the gateway never reported.
fn usage_columns(
    usage: Option<RoundUsage>,
) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    match usage {
        Some(u) => (
            Some(u.prompt_tokens as i64),
            Some(u.completion_tokens as i64),
            Some(u.cached_tokens as i64),
            Some(u.cache_write_tokens as i64),
        ),
        None => (None, None, None, None),
    }
}
