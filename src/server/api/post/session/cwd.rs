//! `/cdp` — the working directory, persisted.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::server::api::driver::Driver;
use crate::server::platform::env;
use crate::server::storage::{Reader, SessionChange, Submit, Writer};

pub(crate) fn apply(driver: &Driver, session: i64, raw: String) -> Result<PathBuf> {
    let cwd = driver.live_cwd(session)?;
    let conn = Reader::open(&driver.db)?;
    let out = crate::commands::Cdp::set(&driver.writer, &conn, session, &cwd, &raw);
    Reader::close(conn);
    let target = out?;
    driver.emit_meta(session)?;
    Ok(target)
}

impl crate::commands::Cdp {
    pub fn set(
        w: &Writer,
        conn: &Connection,
        session: i64,
        cwd: &Path,
        raw: &str,
    ) -> Result<PathBuf> {
        let target = resolve(cwd, raw)?;
        let turn = tip_turn(conn, session)?;
        w.push(Submit::Session {
            session,
            change: SessionChange::Cwd {
                turn,
                path: target.display().to_string(),
            },
        })?;
        Ok(target)
    }
}

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
