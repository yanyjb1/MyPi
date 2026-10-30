//! Session rows and the working-directory history.

use anyhow::Result;

/// A new session row.
pub(crate) fn insert(
    tx: &rusqlite::Transaction<'_>,
    started_at: &str,
    cwd: &str,
    model: Option<&(String, String)>,
    name: Option<&str>,
    anchor: Option<i64>,
    meta: Option<i64>,
) -> Result<i64> {
    let (provider, id) = match model {
        Some((p, m)) => (Some(p.as_str()), Some(m.as_str())),
        None => (None, None),
    };
    tx.execute(
        "INSERT INTO sessions (started_at, cwd, provider, model, name, anchor, meta)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![started_at, cwd, provider, id, name, anchor, meta],
    )?;
    Ok(tx.last_insert_rowid())
}

/// `sessions.name` (`None` = clear it).
pub(crate) fn rename(
    tx: &rusqlite::Transaction<'_>,
    session: i64,
    name: Option<&str>,
) -> Result<()> {
    tx.execute(
        "UPDATE sessions SET name = ?1 WHERE id = ?2",
        rusqlite::params![name, session],
    )?;
    Ok(())
}

/// `sessions.provider` + `sessions.model` (`None` = follow the configured default).
pub(crate) fn switch(
    tx: &rusqlite::Transaction<'_>,
    session: i64,
    model: Option<&(String, String)>,
) -> Result<()> {
    let (p, m) = match model {
        Some((p, m)) => (Some(p.as_str()), Some(m.as_str())),
        None => (None, None),
    };
    tx.execute(
        "UPDATE sessions SET provider = ?1, model = ?2 WHERE id = ?3",
        rusqlite::params![p, m, session],
    )?;
    Ok(())
}

/// `sessions.cwd`.
pub(crate) fn move_cwd(tx: &rusqlite::Transaction<'_>, session: i64, cwd: &str) -> Result<()> {
    tx.execute(
        "UPDATE sessions SET cwd = ?1 WHERE id = ?2",
        rusqlite::params![cwd, session],
    )?;
    Ok(())
}

/// `sessions.todo_on`.
pub(crate) fn todo_on(tx: &rusqlite::Transaction<'_>, session: i64, on: bool) -> Result<()> {
    tx.execute(
        "UPDATE sessions SET todo_on = ?1 WHERE id = ?2",
        rusqlite::params![on as i64, session],
    )?;
    Ok(())
}

/// One `cwd_history` row: this round's working directory.
pub(crate) fn cwd_row(
    tx: &rusqlite::Transaction<'_>,
    session: i64,
    turn: i64,
    cwd: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO cwd_history (session_id, turn_id, cwd) VALUES (?1, ?2, ?3)
         ON CONFLICT(session_id, turn_id) DO UPDATE SET cwd = ?3",
        rusqlite::params![session, turn, cwd],
    )?;
    Ok(())
}
