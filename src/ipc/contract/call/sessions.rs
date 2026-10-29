//! `call.sessions`: the resume list.

use crate::ipc::contract::SessionMeta;

/// One session row on the picker.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionRow {
    pub meta: SessionMeta,
    pub first_user: Option<String>,
    pub bytes: i64,
    pub last_turn: Option<i64>,
}

/// Request: `{under?}`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Ask {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub under: Option<std::path::PathBuf>,
}

/// Reply: `{rows}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    pub rows: Vec<SessionRow>,
}
