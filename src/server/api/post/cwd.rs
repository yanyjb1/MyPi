//! `/cdp` — the working directory, persisted. `~` is expanded here, a relative path resolves against
//! the live directory the driver hands in, and the result must exist.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::server::platform::env;
use crate::server::storage::{SessionChange, Submit, Writer};

impl crate::commands::Cdp {
    /// Resolve `raw` against `cwd` and migrate the session's directory.
    pub fn set(
        w: &Writer,
        conn: &Connection,
        session: i64,
        cwd: &Path,
        raw: &str,
    ) -> Result<()> {
        let target = resolve(cwd, raw)?;
        let turn = tip_turn(conn, session)?;
        w.push(Submit::Session {
            session,
            change: SessionChange::Cwd {
                turn,
                path: target.display().to_string(),
            },
        })?;
        Ok(())
    }
}

/// A `/cdp` argument: `~` expanded, relative to `cwd`, must exist.
fn resolve(cwd: &Path, raw: &str) -> Result<PathBuf> {
    let raw = raw.trim();
    anyhow::ensure!(!raw.is_empty(), "usage: /cdp <dir>");
    let p = match raw.strip_prefix('~') {
        Some(rest) => env::home().unwrap_or_default().join(rest),
        None => PathBuf::from(raw),
    };
    let p = if p.is_absolute() { p } else { cwd.join(p) };
    let real = p
        .canonicalize()
        .with_context(|| format!("no such directory: {raw}"))?;
    anyhow::ensure!(real.is_dir(), "not a directory: {raw}");
    Ok(real)
}

/// The round history should be anchored on: the last item of the session's
/// list (`None` = the session has no rounds yet).
fn tip_turn(conn: &Connection, session: i64) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT turn_id FROM session_turns WHERE session_id = ?1
              ORDER BY idx DESC LIMIT 1",
            [session],
            |r| r.get(0),
        )
        .ok()
        .flatten())
}
