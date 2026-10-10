//! Deleting a whole session: its list, its working-directory history, its row, then the sweep.
//! Shared blocks survive; request archives stay. **When** belongs to the api.

use anyhow::Result;

use super::prune;
use super::super::writer::Store;

impl Store {
    /// Remove one session and everything only it referenced.
    pub fn delete_session(&mut self, session: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM session_turns WHERE session_id = ?1", [session])?;
        tx.execute("DELETE FROM cwd_history WHERE session_id = ?1", [session])?;
        tx.execute("DELETE FROM sessions WHERE id = ?1", [session])?;
        prune(&tx)?;
        tx.commit()?;
        Ok(())
    }
}
