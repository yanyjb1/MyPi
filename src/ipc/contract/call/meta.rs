//! `call.meta`: the session's row and its live cwd.

use std::path::PathBuf;

use crate::ipc::contract::SessionMeta;

/// Request: `{session}`.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Ask {
    pub session: i64,
}

/// Reply: `{meta, cwd}`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    pub meta: SessionMeta,
    pub cwd: PathBuf,
}
