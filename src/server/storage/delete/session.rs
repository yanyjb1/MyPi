//! Deleting a whole session: its list, its working-directory history, its row, then the sweep.
//! Shared blocks survive; request archives stay. **When** belongs to the api.

use anyhow::Result;
use rusqlite::params;

use super::super::write::shared::session_blocks;
use super::super::writer::Store;
use super::{export, prune};

impl Store {
    /// Remove one session and everything only it referenced.
    pub fn delete_session(&mut self, session: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        export::current(&tx, session)?;
        let ranges = session_blocks(&tx, session)?;
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS doomed_blocks (id INTEGER PRIMARY KEY);
             DELETE FROM doomed_blocks;",
        )?;
        for [lo, hi] in ranges {
            tx.execute(
                "WITH RECURSIVE r(x) AS (SELECT ?1 UNION ALL SELECT x + 1 FROM r WHERE x < ?2)
                 INSERT OR IGNORE INTO doomed_blocks (id) SELECT x FROM r",
                params![lo, hi],
            )?;
        }
        tx.execute(
            "DELETE FROM memos WHERE anchor IN (SELECT id FROM doomed_blocks)",
            [],
        )?;
        tx.execute(
            "DELETE FROM aliases WHERE anchor IN (SELECT id FROM doomed_blocks)",
            [],
        )?;
        tx.execute(
            "DELETE FROM subsessions
              WHERE subsession_id = ?1 OR anchor IN (SELECT id FROM doomed_blocks)",
            [session],
        )?;
        tx.execute("DROP TABLE doomed_blocks", [])?;
        tx.execute("DELETE FROM session_turns WHERE session_id = ?1", [session])?;
        tx.execute("DELETE FROM cwd_history WHERE session_id = ?1", [session])?;
        tx.execute("DELETE FROM sessions WHERE id = ?1", [session])?;
        prune(&tx)?;
        tx.commit()?;
        Ok(())
    }
}
