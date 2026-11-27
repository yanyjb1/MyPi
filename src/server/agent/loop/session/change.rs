//! One field at a time, as the caller named it.

use std::path::PathBuf;

use anyhow::{Result, bail};

use super::handle::Handle;
use crate::server::agent::tools::contract::env::{Defaults, roster};
use crate::server::storage::{Row, Submit, Writer};

/// One session-scoped change.
#[derive(Debug, Clone)]
pub enum Change {
    Model {
        provider: String,
        model: String,
        /// Whether the new model takes image parts; a switch reaches the tool side too.
        vision: bool,
    },
    Cwd(PathBuf),
    /// Purely in memory: an effort never reaches the rows.
    Effort(crate::server::config::Effort),
}

pub(crate) fn apply(
    handle: &Handle,
    change: Change,
    writer: &Writer,
    scrooge: bool,
    defaults: &Defaults,
) -> Result<Option<PathBuf>> {
    match change {
        Change::Model {
            provider,
            model,
            vision,
        } => {
            if scrooge {
                bail!("this loop does not allow a model change mid-session");
            }
            {
                let mut config = handle.config_mut();
                config.provider = provider.clone();
                config.model = model.clone();
                config.effort = crate::server::config::Effort::Auto;
            }
            handle.env().vision = vision;
            handle.env().models = roster(&defaults.models, &format!("{provider}:{model}"));
            writer.push(Submit {
                rows: vec![Row::Switch {
                    session: handle.id,
                    model: Some((provider, model)),
                }],
            })?;
        }
        Change::Effort(effort) => {
            handle.config_mut().effort = effort;
        }
        Change::Cwd(path) => {
            // `/cdp` asks only whether the directory exists; `cd` answers where it may stand.
            let base = handle.cwd();
            let real = crate::server::agent::tools::workspace::resolve_dir(
                &base,
                &path.to_string_lossy(),
            )?;
            handle.chdir(real.clone());
            // A move that lands where the session already stands is no row change.
            if real != base {
                writer.push(Submit {
                    rows: vec![Row::MoveCwd {
                        session: handle.id,
                        cwd: real.to_string_lossy().into_owned(),
                    }],
                })?;
            }
            return Ok(Some(real));
        }
    }
    Ok(None)
}
