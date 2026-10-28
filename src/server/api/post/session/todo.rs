//! Todo on/off: the front end's flag and the toolresult marker, in one write.

use anyhow::Result;

use crate::server::api::driver::Driver;
use crate::server::storage::{SessionChange, Submit};

pub(crate) fn apply(driver: &Driver, session: i64, on: bool, call_id: String) -> Result<()> {
    driver.writer.push(Submit::Session {
        session,
        change: SessionChange::Todo { on, call_id },
    })?;
    Ok(())
}
