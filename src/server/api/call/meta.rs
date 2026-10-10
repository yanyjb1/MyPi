//! The session's metadata: the row's columns, plus the in-memory working directory the driver hands
//! in (no query can produce it).

use anyhow::Result;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::server::storage::SessionMeta;
use crate::server::storage::decode::session_meta;

use super::sessions::SESSION_COLS;

/// One session as it stands: its row, and the directory it is working in.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionState {
    /// The session row's columns.
    pub meta: SessionMeta,
    /// Where the session is working right now.
    pub cwd: PathBuf,
}

/// Read the row and attach `cwd`.
pub fn state(conn: &Connection, session: i64, cwd: &Path) -> Result<SessionState> {
    let meta = conn.query_row(
        &format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"),
        [session],
        session_meta,
    )?;
    Ok(SessionState {
        meta,
        cwd: cwd.to_path_buf(),
    })
}
