//! `/name` — the session's display name.

use anyhow::Result;

use crate::server::api::driver::Driver;
use crate::server::storage::{SessionChange, Submit, Writer};

pub(crate) fn apply(driver: &Driver, session: i64, name: Option<String>) -> Result<()> {
    crate::commands::Name::set(&driver.writer, session, name.as_deref())?;
    driver.emit_meta(session)
}

impl crate::commands::Name {
    pub fn set(w: &Writer, session: i64, name: Option<&str>) -> Result<()> {
        w.push(Submit::Session {
            session,
            change: SessionChange::Name(name.map(str::to_string)),
        })?;
        Ok(())
    }
}
