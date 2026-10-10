//! Session rows: create one, rename it, point it at a model, migrate it.

use anyhow::Result;

use super::super::writer::Store;

impl Store {
    /// Create a session row, returning its id.
    pub(crate) fn create_session(&mut self, started_at: &str, cwd: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO sessions (started_at, cwd) VALUES (?1, ?2)",
            rusqlite::params![started_at, cwd],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Create a session row on its own, with the values the front end collected.
    pub(crate) fn open_session(
        &mut self,
        started_at: &str,
        cwd: &str,
        model: Option<(&str, &str)>,
        name: Option<&str>,
    ) -> Result<i64> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO sessions (started_at, cwd) VALUES (?1, ?2)",
            rusqlite::params![started_at, cwd],
        )?;
        let id = tx.last_insert_rowid();
        if let Some(name) = name {
            tx.execute(
                "UPDATE sessions SET name = ?1 WHERE id = ?2",
                rusqlite::params![name, id],
            )?;
        }
        if let Some((provider, model)) = model {
            tx.execute(
                "UPDATE sessions SET model_provider = ?1, model = ?2 WHERE id = ?3",
                rusqlite::params![provider, model, id],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// `/name`: write the name (`None` = clear it).
    pub(crate) fn set_session_name(&mut self, session_id: i64, name: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, session_id],
        )?;
        Ok(())
    }

    /// Which model this session is on (`None` = follow the configured
    /// default).
    pub(crate) fn set_session_model(
        &mut self,
        session_id: i64,
        model: Option<(&str, &str)>,
    ) -> Result<()> {
        let (p, m) = match model {
            Some((p, m)) => (Some(p), Some(m)),
            None => (None, None),
        };
        self.conn.execute(
            "UPDATE sessions SET model_provider = ?1, model = ?2 WHERE id = ?3",
            rusqlite::params![p, m, session_id],
        )?;
        Ok(())
    }

    /// `/cdp`: migrate the session's working directory. Two things land: the session row's `cwd`
    /// and the round-anchored `cwd_history` entry.
    pub(crate) fn set_session_cwd(
        &mut self,
        session_id: i64,
        turn: Option<i64>,
        path: &str,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE sessions SET cwd = ?1 WHERE id = ?2",
            rusqlite::params![path, session_id],
        )?;
        if let Some(turn) = turn {
            tx.execute(
                "INSERT INTO cwd_history (session_id, turn_id, cwd) VALUES (?1, ?2, ?3)
                 ON CONFLICT(session_id, turn_id) DO UPDATE SET cwd = ?3",
                rusqlite::params![session_id, turn, path],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
