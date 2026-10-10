//! Creating a session before any message: the id has to come back before a round can open, which is
//! why `Post::Init` is the one post that answers.

use anyhow::{Context, Result};
use std::path::Path;

use crate::server::api::types::ModelId;
use crate::server::config::Config;
use crate::server::storage::{Submit, Writer};

/// Create the session row and answer with its id. The model is checked against the config first.
pub fn create(
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
    })?;
    landed.session.context("the writer created no session row")
}
