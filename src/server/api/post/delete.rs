//! `/delete` (alias `/rm`) — remove one session, named by its id. The sweep itself is storage's.

use anyhow::Result;

use crate::server::storage::Writer;

impl crate::commands::Delete {
    /// Remove the session.
    pub fn remove(w: &Writer, session: i64) -> Result<()> {
        w.delete_session(session)
    }
}
