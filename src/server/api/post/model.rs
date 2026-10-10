//! `/switch` and `/model` — the session's model, and the configured default. Both check the address
//! first: storing a typo would fail on the next request.

use anyhow::Result;

use crate::server::api::types::ModelId;
use crate::server::config::Config;
use crate::server::storage::{SessionChange, Submit, Writer};

impl crate::commands::Switch {
    /// Point this session at `id`.
    pub fn set(cfg: &Config, w: &Writer, session: i64, id: &ModelId) -> Result<()> {
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

impl crate::commands::Model {
    /// Persist `id` as the default model. The file it landed in is the
    /// caller's to report; this layer only guarantees the write happened.
    pub fn set_default(cfg: &Config, id: &ModelId) -> Result<()> {
        cfg.set_default_model(&id.addressed())?;
        Ok(())
    }
}
