//! `call.children`: the child sessions one session spawned.

/// Where a child stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildState {
    Queued,
    Running,
    Ended,
}

/// One child session.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChildRow {
    pub anchor: i64,
    pub session: crate::ipc::contract::SessionRow,
    pub intent: Option<String>,
    pub state: ChildState,
}

/// Request: `{session}`.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Ask {
    pub session: i64,
}

/// Reply: `{rows}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    pub rows: Vec<ChildRow>,
}
