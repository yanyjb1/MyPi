//! `call.blocks`: landed blocks by closed id ranges.

/// Request: `{session, ranges}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Ask {
    pub session: i64,
    pub ranges: Vec<[i64; 2]>,
}

/// Reply: `{blocks}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    pub blocks: Vec<crate::ipc::contract::Block>,
}
