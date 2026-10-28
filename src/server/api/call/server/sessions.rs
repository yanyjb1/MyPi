//! Session rows: the resume list, one row by id, and the columns both share.

use anyhow::Result;
use rusqlite::Connection;

use crate::server::api::convert;
use crate::server::storage::decode::{TURN_COLS, session_meta};
use crate::server::storage::{SessionMeta, TurnRow};

use crate::server::api::call::session::{blocks_of, read_start, user_text};

/// The session columns, in `decode::session_meta` order.
pub(crate) const SESSION_COLS: &str =
    "id, name, started_at, cwd, model_provider, model, profile, todo_on";

pub use crate::ipc::contract::{SessionRow, display_name};

/// The picker's rows, newest first; child sessions are not sessions of their own.
pub(crate) fn rows(conn: &Connection, under: Option<&std::path::Path>) -> Result<Vec<SessionRow>> {
    let under = under.map(|p| p.to_string_lossy().to_string());
    let mut stmt = conn.prepare(&format!(
        "SELECT {SESSION_COLS}
           FROM sessions s
          WHERE (?1 IS NULL
                 OR s.id IN (SELECT DISTINCT session_id FROM cwd_history WHERE cwd = ?1))
            AND s.id NOT IN (SELECT subsession_id FROM subsessions)
          ORDER BY s.id DESC"
    ))?;
    let found = stmt.query_map(rusqlite::params![under], session_meta)?;
    let mut out = Vec::new();
    for meta in found {
        out.push(build(conn, meta?)?);
    }
    Ok(out)
}

/// One row by id.
pub(crate) fn row(conn: &Connection, session: i64) -> Result<SessionRow> {
    let meta = conn.query_row(
        &format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"),
        [session],
        session_meta,
    )?;
    build(conn, meta)
}

/// One session's row columns.
pub(crate) fn meta(conn: &Connection, session: i64) -> Result<SessionMeta> {
    Ok(conn.query_row(
        &format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"),
        [session],
        session_meta,
    )?)
}

fn build(conn: &Connection, meta: SessionMeta) -> Result<SessionRow> {
    let first_user = first_user(conn, meta.id)?;
    let bytes = size(conn, meta.id)?;
    let last_turn = last_turn(conn, meta.id)?;
    Ok(SessionRow {
        meta: convert::session_meta(meta),
        first_user,
        bytes,
        last_turn,
    })
}

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
