//! `/switch` — this session's model.

use anyhow::Result;

use crate::ipc::contract::post::ModelId;
use crate::server::api::driver::Driver;
use crate::server::storage::{SessionChange, Submit, Writer};

pub(crate) fn apply(driver: &Driver, session: i64, id: ModelId) -> Result<()> {
    crate::commands::Switch::set(&driver.cfg, &driver.writer, session, &id)?;
    driver
        .sessions
        .retarget(&driver.cfg, session, &id.addressed())?;
    driver.emit_meta(session)
}

impl crate::commands::Switch {
    pub fn set(
        cfg: &crate::server::config::Config,
        w: &Writer,
        session: i64,
        id: &ModelId,
    ) -> Result<()> {
        let resolved = cfg.model_by_id(&id.addressed())?;
        w.push(Submit::Session {
            session,
            change: SessionChange::Model(Some((
                resolved.provider_name.clone(),
                resolved.entry.id.clone(),
            ))),
        })?;
        Ok(())
    }
}
