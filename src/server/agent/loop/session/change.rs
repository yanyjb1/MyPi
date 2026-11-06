//! One field at a time, already resolved by the caller.

use std::path::PathBuf;

use anyhow::{Result, bail};

use super::handle::Handle;
use crate::server::storage::{Row, Submit, Writer};

/// One session-scoped change.
#[derive(Debug, Clone)]
pub enum Change {
    Model { provider: String, model: String },
    Cwd(PathBuf),
}

pub(crate) fn apply(
    handle: &Handle,
    change: Change,
    writer: &Writer,
    scrooge: bool,
) -> Result<Option<PathBuf>> {
    match change {
        Change::Model { provider, model } => {
            if scrooge {
                bail!("this loop does not allow a model change mid-session");
            }
            {
                let mut birth = handle.birth.lock().unwrap_or_else(|e| e.into_inner());
                birth.provider = provider.clone();
                birth.model = model.clone();
            }
            if let Some(live) = handle
                .live
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
            {
                live.provider = provider.clone();
                live.model = model.clone();
            }
            writer.push(Submit {
                rows: vec![Row::Switch {
                    session: handle.id,
                    model: Some((provider, model)),
                }],
            })?;
        }
        Change::Cwd(path) => {
            // `/cdp` asks only whether the directory exists; `cd` answers where it may stand.
            let base = match handle
                .live
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                Some(live) => live.cwd.read().unwrap_or_else(|e| e.into_inner()).clone(),
                None => handle.birth().cwd.clone().unwrap_or_default(),
            };
            let real = crate::server::agent::tools::workspace::resolve_dir(
                &base,
                &path.to_string_lossy(),
            )?;
            if let Some(live) = handle
                .live
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
            {
                *live.cwd.write().unwrap_or_else(|e| e.into_inner()) = real.clone();
            }
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
