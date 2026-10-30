//! One field at a time, already resolved by the caller.

use std::path::PathBuf;

use anyhow::{Result, bail};

use super::handle::Handle;
use crate::server::storage::{Ref, RequestMeta, Row, Submit, Writer};

/// One session-scoped change.
#[derive(Debug, Clone)]
pub enum Change {
    Model { provider: String, model: String },
    Cwd(PathBuf),
}

pub(crate) fn apply(handle: &Handle, change: Change, writer: &Writer, scrooge: bool) -> Result<()> {
    match change {
        Change::Model { provider, model } => {
            if scrooge {
                bail!("this loop does not allow a model change mid-session");
            }
            let birth = {
                let mut birth = handle.birth.lock().unwrap_or_else(|e| e.into_inner());
                birth.provider = provider.clone();
                birth.model = model.clone();
                birth.clone()
            };
            if let Some(live) = handle
                .live
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
            {
                live.provider = provider.clone();
                live.model = model.clone();
            }
            let tools_json =
                serde_json::to_string(&birth.tools).unwrap_or_else(|_| "[]".to_string());
            let landed = writer.push(Submit {
                rows: vec![
                    Row::Request {
                        meta: RequestMeta {
                            model: model.clone(),
                            provider: provider.clone(),
                            system: birth.system,
                            tools_json,
                            baseline_prompt: None,
                        },
                    },
                    Row::Switch {
                        session: handle.id,
                        model: Some((provider, model)),
                    },
                    Row::Meta {
                        session: handle.id,
                        meta: Some(Ref::Batch(0)),
                    },
                ],
            })?;
            if let Some(meta) = landed.ids.first().copied().filter(|id| *id > 0) {
                let mut birth = handle.birth.lock().unwrap_or_else(|e| e.into_inner());
                birth.meta = Some(meta);
            }
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
                live.cwd = path.clone();
            }
            writer.push(Submit {
                rows: vec![Row::MoveCwd {
                    session: handle.id,
                    cwd: path.to_string_lossy().into_owned(),
                }],
            })?;
        }
    }
    Ok(())
}
