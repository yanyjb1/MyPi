//! The read recipe: how to open a read connection, and how to close it. Storage answers no query of
//! its own; a reader writes its own SQL and decodes the rows.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// The read side's entry point.
pub struct Reader;

impl Reader {
    /// Open a read connection. The database must exist: the writer's
    /// [`open`](super::Writer::open) creates it.
    pub fn open(path: &std::path::Path) -> Result<Connection> {
        let conn = Connection::open(path)
            .with_context(|| format!("cannot open database {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(conn)
    }

    /// Close it. Read once, close; reading continuously, keep it and close
    /// when the reading is over.
    pub fn close(conn: Connection) {
        drop(conn);
    }
}
