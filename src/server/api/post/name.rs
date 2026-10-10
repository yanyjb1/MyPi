//! `/name` — the session's display name. Its one home is `sessions.name`.

use anyhow::Result;

use crate::server::storage::{SessionChange, Submit, Writer};

impl crate::commands::Name {
    /// Set the display name; `None` clears it.
    pub fn set(w: &Writer, session: i64, name: Option<&str>) -> Result<()> {
        w.push(Submit::Session {
            session,
            change: SessionChange::Name(name.map(str::to_string)),
        })?;
        Ok(())
    }
}
