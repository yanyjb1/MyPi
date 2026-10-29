//! `call.init`: the session's first frame.

use crate::ipc::contract::{Block, TodoPhase};

/// One round's place in the structure, newest first.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TurnShape {
    pub turn: i64,
    pub blocks: Vec<TurnBlock>,
}

/// One block's id and the hash that claims it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TurnBlock {
    pub id: i64,
    pub hash: u64,
}

/// Request: `{session, limit}`.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Ask {
    pub session: i64,
    pub limit: usize,
}

/// Reply: `{structure, blocks, todo?}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    pub structure: Vec<TurnShape>,
    pub blocks: Vec<Block>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo: Option<Vec<TodoPhase>>,
}
