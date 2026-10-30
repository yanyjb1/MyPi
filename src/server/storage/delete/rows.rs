//! Row deletion: the one way rows leave the database, followed by the sweep.

use anyhow::Result;

use super::super::shape::Table;
use super::super::writer::Store;
use super::sweep;

impl Store {
    /// Delete these rows of one table, with whatever only they held, then sweep.
    pub fn delete_rows(&mut self, table: Table, ids: &[i64]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut removed = 0;
        for id in ids {
            removed += remove(&tx, table, *id)?;
        }
        sweep::run(&tx)?;
        tx.commit()?;
        Ok(removed)
    }
}

/// One row, plus the rows that reference it and nothing else.
pub(crate) fn remove(tx: &rusqlite::Transaction<'_>, table: Table, id: i64) -> Result<usize> {
    Ok(match table {
        Table::Sessions => {
            tx.execute("DELETE FROM session_turns WHERE session_id = ?1", [id])?;
            tx.execute("DELETE FROM cwd_history WHERE session_id = ?1", [id])?;
            tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?
        }
        Table::Turns => {
            tx.execute("DELETE FROM session_turns WHERE turn_id = ?1", [id])?;
            tx.execute("DELETE FROM turns WHERE turn_id = ?1", [id])?
        }
        Table::Artifacts => tx.execute("DELETE FROM artifacts WHERE block_id = ?1", [id])?,
        Table::Memos => tx.execute("DELETE FROM memos WHERE id = ?1", [id])?,
        Table::Aliases => tx.execute("DELETE FROM aliases WHERE id = ?1", [id])?,
    })
}
