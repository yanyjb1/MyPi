//! Session queries: the picker's rows. Everything a picker draws is computed in this module and
//! nowhere else.

use anyhow::Result;
use rusqlite::Connection;

use crate::server::storage::decode::{TURN_COLS, session_meta};
use crate::server::storage::{SessionMeta, TurnRow};

use super::{blocks_of, read_start, user_text};

/// The session columns, in the order `decode::session_meta` reads them: the picker's rows and
/// `meta.rs`'s single row share these seven.
pub(crate) const SESSION_COLS: &str = "id, name, started_at, cwd, model_provider, model, todo_on";

/// One picker row: session metadata plus the few things a table does not
/// carry but a UI draws.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionRow {
    pub meta: SessionMeta,
    /// The first user message (the preview line, and the fallback display name
    /// for an unnamed session).
    pub first_user: Option<String>,
    /// The sum of the list's (from the read start) round `bytes`.
    pub bytes: i64,
    /// The last list item's round id (`None` = empty session).
    pub last_turn: Option<i64>,
}

/// The picker's rows, newest first. `under` narrows to sessions that worked
/// in that directory (project scope); `None` = all.
pub fn rows(conn: &Connection, under: Option<&std::path::Path>) -> Result<Vec<SessionRow>> {
    let under = under.map(|p| p.to_string_lossy().to_string());
    let mut stmt = conn.prepare(&format!(
        "SELECT {SESSION_COLS}
           FROM sessions s
          WHERE (?1 IS NULL
                 OR s.id IN (SELECT DISTINCT session_id FROM cwd_history WHERE cwd = ?1))
          ORDER BY s.id DESC"
    ))?;
    let found = stmt.query_map(rusqlite::params![under], session_meta)?;
    let mut out = Vec::new();
    for meta in found {
        let meta = meta?;
        let first_user = first_user(conn, meta.id)?;
        let bytes = size(conn, meta.id)?;
        let last_turn = last_turn(conn, meta.id)?;
        out.push(SessionRow {
            meta,
            first_user,
            bytes,
            last_turn,
        });
    }
    Ok(out)
}

/// Display name in the resume picker: the stored name, else
/// `MM-DD:HH-MMSS+first 7 chars`.
pub fn display_name(meta: &SessionMeta, first_user: Option<&str>) -> String {
    if let Some(n) = &meta.name {
        return n.clone();
    }
    let prefix = first_user
        .map(|s| s.chars().take(7).collect::<String>())
        .unwrap_or_default();
    // "2026-09-22 14:30:05" → "09-22:14-3005"; sliced by characters, so a truncated row cannot panic.
    let t: Vec<char> = meta.started_at.chars().collect();
    let part = |a: usize, b: usize| t[a..b].iter().collect::<String>();
    if t.len() >= 19 {
        format!(
            "{}-{}:{}-{}{}+{}",
            part(5, 7),
            part(8, 10),
            part(11, 13),
            part(14, 16),
            part(17, 19),
            prefix
        )
    } else {
        format!("?+{prefix}")
    }
}

/// The session's size: the sum of round `bytes` from the read start.
/// Addition only; blocks untouched.
fn size(conn: &Connection, session_id: i64) -> Result<i64> {
    let from = read_start(conn, session_id)?.unwrap_or(0);
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(t.bytes), 0) FROM session_turns s
           JOIN turns t ON t.turn_id = s.turn_id
          WHERE s.session_id = ?1 AND s.idx >= ?2",
        rusqlite::params![session_id, from],
        |r| r.get(0),
    )?)
}

/// The last list item's round id.
fn last_turn(conn: &Connection, session_id: i64) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT turn_id FROM session_turns WHERE session_id = ?1
              ORDER BY idx DESC LIMIT 1",
            [session_id],
            |r| r.get(0),
        )
        .ok()
        .flatten())
}

/// The preview line: the list's **first user message**, found by walking the list from the head
/// (the block carries the kind already).
fn first_user(conn: &Connection, session_id: i64) -> Result<Option<String>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TURN_COLS} FROM session_turns s
           JOIN turns t ON t.turn_id = s.turn_id
           LEFT JOIN request_meta r ON r.request_id = t.request_id
          WHERE s.session_id = ?1 ORDER BY s.idx"
    ))?;
    let found = stmt.query_map([session_id], crate::server::storage::decode::turn_from_row)?;
    for row in found {
        let turn: TurnRow = row?;
        for block in blocks_of(conn, &turn)? {
            if let Some(text) = user_text(&block) {
                return Ok(Some(text));
            }
        }
    }
    Ok(None)
}
