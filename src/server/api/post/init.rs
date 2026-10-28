//! Creating a session before any message: the id has to come back first.

use anyhow::{Context, Result};
use std::path::Path;

use crate::server::config::Config;
use crate::server::storage::{Submit, Writer};

use super::ModelId;

pub(crate) fn create(
    cfg: &Config,
    w: &Writer,
    cwd: &Path,
    model: Option<&ModelId>,
    name: Option<&str>,
) -> Result<i64> {
    let model = match model {
        Some(id) => {
            let resolved = cfg.model_by_id(&id.addressed())?;
            Some((resolved.provider_name.clone(), resolved.entry.id.clone()))
        }
        None => None,
    };
    let landed = w.push(Submit::NewSession {
        started_at: crate::server::platform::time::now_stamp(),
        cwd: cwd.display().to_string(),
        model,
        name: name.map(str::to_string),
        profile: None,
    })?;
    landed.session.context("the writer created no session row")
}
