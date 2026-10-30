//! Small SQL helpers the write half shares between its own tables.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use super::super::shape::RequestMeta;

/// Append one item at the list tail (`idx` is computed here).
pub(crate) fn append_list_row(
    tx: &rusqlite::Transaction<'_>,
    session_id: i64,
    turn_id: i64,
    is_compaction: bool,
) -> Result<i64> {
    let idx: i64 = tx.query_row(
        "SELECT COALESCE(MAX(idx) + 1, 0) FROM session_turns WHERE session_id = ?1",
        [session_id],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO session_turns (session_id, idx, turn_id, is_compaction)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![session_id, idx, turn_id, is_compaction as i64],
    )?;
    Ok(idx)
}

/// Every block interval the session's list holds.
pub(crate) fn session_blocks(conn: &Connection, session: i64) -> Result<Vec<[i64; 2]>> {
    let mut stmt = conn.prepare(
        "SELECT t.blocks FROM session_turns s JOIN turns t ON t.turn_id = s.turn_id
          WHERE s.session_id = ?1 ORDER BY s.idx",
    )?;
    let rows = stmt.query_map([session], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for raw in rows {
        out.extend(super::super::decode::parse_blocks(&raw?)?);
    }
    Ok(out)
}

/// Is this id inside one of the intervals?
pub(crate) fn in_ranges(ranges: &[[i64; 2]], id: i64) -> bool {
    ranges.iter().any(|[lo, hi]| *lo <= id && id <= *hi)
}

/// The session a round belongs to (`None` = the round is not in any list).
pub(crate) fn session_of_turn(conn: &Connection, turn_id: i64) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT session_id FROM session_turns WHERE turn_id = ?1 LIMIT 1",
            [turn_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Point a list place at another round.
pub(crate) fn set_list_turn(
    tx: &rusqlite::Transaction<'_>,
    session: i64,
    idx: i64,
    turn: i64,
) -> Result<()> {
    tx.execute(
        "UPDATE session_turns SET turn_id = ?1, is_compaction = 0
          WHERE session_id = ?2 AND idx = ?3",
        rusqlite::params![turn, session, idx],
    )?;
    Ok(())
}

/// Drop the list places from `idx` on.
pub(crate) fn delete_list_from(
    tx: &rusqlite::Transaction<'_>,
    session: i64,
    idx: i64,
) -> Result<usize> {
    Ok(tx.execute(
        "DELETE FROM session_turns WHERE session_id = ?1 AND idx >= ?2",
        rusqlite::params![session, idx],
    )?)
}

/// Drop the list places after `idx`.
pub(crate) fn delete_list_after(
    tx: &rusqlite::Transaction<'_>,
    session: i64,
    idx: i64,
) -> Result<usize> {
    Ok(tx.execute(
        "DELETE FROM session_turns WHERE session_id = ?1 AND idx > ?2",
        rusqlite::params![session, idx],
    )?)
}

/// One round row.
pub(crate) fn read_turn(
    conn: &Connection,
    turn_id: i64,
) -> Result<Option<super::super::shape::TurnRow>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {} {} WHERE t.turn_id = ?1",
                super::super::decode::TURN_COLS,
                super::super::decode::TURN_FROM
            ),
            [turn_id],
            super::super::decode::turn_from_row,
        )
        .optional()?)
}

/// Write a request archive row (deduplicated by content hash), returning its `request_id`.
pub(crate) fn put_request_meta(tx: &rusqlite::Transaction<'_>, r: &RequestMeta) -> Result<i64> {
    let hash = request_hash(r);
    if let Some(id) = tx
        .query_row(
            "SELECT request_id FROM request_meta WHERE hash = ?1",
            [&hash],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    tx.execute(
        "INSERT INTO request_meta
            (hash, model, provider, system, tools_json, baseline_prompt)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            hash,
            r.model,
            r.provider,
            r.system,
            r.tools_json,
            r.baseline_prompt.map(|v| v as i64)
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

/// Point one session at an archive, or clear the pointer.
pub(crate) fn set_meta(
    tx: &rusqlite::Transaction<'_>,
    session: i64,
    meta: Option<i64>,
) -> Result<()> {
    tx.execute(
        "UPDATE sessions SET meta = ?1 WHERE id = ?2",
        rusqlite::params![meta, session],
    )?;
    Ok(())
}

/// Lower one archive's prompt baseline; it never grows.
pub(crate) fn lower_baseline(
    tx: &rusqlite::Transaction<'_>,
    request: i64,
    prompt: u64,
) -> Result<()> {
    tx.execute(
        "UPDATE request_meta SET baseline_prompt = ?1
          WHERE request_id = ?2
            AND (baseline_prompt IS NULL OR baseline_prompt > ?1)",
        rusqlite::params![prompt as i64, request],
    )?;
    Ok(())
}

/// The request archive's fingerprint: FNV-1a 64-bit plus each field's length, so concatenation is
/// unambiguous. The hash is stored, not the text.
fn request_hash(r: &RequestMeta) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in (bytes.len() as u64).to_be_bytes().iter().chain(bytes) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    };
    eat(r.model.as_bytes());
    eat(r.provider.as_bytes());
    eat(r.system.as_bytes());
    eat(r.tools_json.as_bytes());
    format!("{h:016x}")
}
