//! Small SQL helpers the write half shares between its own steps.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use super::super::decode::{TURN_COLS, TURN_FROM, parse_blocks};
use super::super::shape::{RequestMeta, TurnRow};

/// Read one round row (inside a transaction, for the write path's own use).
pub(crate) fn read_turn(conn: &Connection, turn_id: i64) -> Result<Option<TurnRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TURN_COLS} {TURN_FROM} WHERE t.turn_id = ?1"
    ))?;
    let mut rows = stmt.query_map([turn_id], super::super::decode::turn_from_row)?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// The round this session is writing into: its last **own** list item
/// (`None` = no round is open — the user's message must land first).
pub(crate) fn open_turn(conn: &Connection, session: i64) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT turn_id FROM session_turns WHERE session_id = ?1 AND kind = 'own'
              ORDER BY idx DESC LIMIT 1",
            [session],
            |r| r.get(0),
        )
        .optional()?)
}

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
        "INSERT INTO session_turns (session_id, idx, turn_id, kind, is_compaction)
         VALUES (?1, ?2, ?3, 'own', ?4)",
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
        out.extend(parse_blocks(&raw?)?);
    }
    Ok(out)
}

/// Is this id inside one of the intervals?
pub(crate) fn in_ranges(ranges: &[[i64; 2]], id: i64) -> bool {
    ranges.iter().any(|[lo, hi]| *lo <= id && id <= *hi)
}

/// Write a request archive row (deduplicated by content hash), returning its
/// `request_id`.
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
            (hash, model, provider, protocol, base_url, profile, system, tools_json, max_tokens,
             compat_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            hash,
            r.model,
            r.provider,
            r.protocol,
            r.base_url,
            r.profile,
            r.system,
            r.tools_json,
            r.max_tokens as i64,
            r.compat_json
        ],
    )?;
    Ok(tx.last_insert_rowid())
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
    eat(r.protocol.as_bytes());
    eat(r.base_url.as_bytes());
    eat(r.profile.as_bytes());
    eat(r.system.as_bytes());
    eat(r.tools_json.as_bytes());
    eat(&(r.max_tokens as u64).to_be_bytes());
    eat(r.compat_json.as_deref().unwrap_or("\u{0}none").as_bytes());
    format!("{h:016x}")
}
