//! One field at a time, already resolved by the caller.

use std::path::PathBuf;

use anyhow::{Result, bail};

use super::handle::Handle;
use crate::server::storage::{Row, Submit, Writer};

/// One session-scoped change.
#[derive(Debug, Clone)]
pub enum Change {
    Model {
        provider: String,
        model: String,
    },
    Cwd(PathBuf),
    /// The prompt a session has not used yet: memory only, never a row.
    Profile {
        system: String,
        tools: Vec<String>,
        compact: String,
    },
}

pub(crate) fn apply(handle: &Handle, change: Change, writer: &Writer, scrooge: bool) -> Result<()> {
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
            {
                let mut birth = handle.birth.lock().unwrap_or_else(|e| e.into_inner());
                birth.cwd = Some(path.clone());
            }
            if let Some(live) = handle
                .live
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
            {
                live.cwd
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone_from(&path);
            }
            writer.push(Submit {
                rows: vec![Row::MoveCwd {
                    session: handle.id,
                    cwd: path.to_string_lossy().into_owned(),
                }],
            })?;
        }
        Change::Profile {
            system,
            tools,
            compact,
        } => {
            {
                let mut birth = handle.birth.lock().unwrap_or_else(|e| e.into_inner());
                birth.system = system.clone();
                birth.tools = tools.clone();
                birth.compact = compact.clone();
            }
            if let Some(live) = handle
                .live
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
            {
                live.system = system;
                live.tools = tools;
                live.compact = compact;
            }
        }
    }
    Ok(())
}
