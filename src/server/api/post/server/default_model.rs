//! `/model` — persist the default model.

use anyhow::Result;

use crate::ipc::contract::post::ModelId;
use crate::server::api::driver::Driver;

pub(crate) fn apply(driver: &Driver, id: ModelId) -> Result<()> {
    crate::commands::Model::set_default(&driver.cfg, &id)?;
    Ok(())
}

impl crate::commands::Model {
    pub fn set_default(cfg: &crate::server::config::Config, id: &ModelId) -> Result<()> {
        cfg.set_default_model(&id.addressed())?;
        Ok(())
    }
}
