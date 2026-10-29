//! `call.todo`: the session's todo list.

use crate::ipc::contract::TodoPhase;

/// Request: `{session}`.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Ask {
    pub session: i64,
}

/// Reply: `{phases}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    pub phases: Vec<TodoPhase>,
}
