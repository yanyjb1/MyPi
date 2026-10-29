//! `call.memo`: memo reads.

/// What a memo read does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoOp {
    List,
    Show,
    History,
}

/// Where a memo lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoScope {
    Current,
    Global,
}

/// One stored version of a memo.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemoEntry {
    pub id: i64,
    pub content: String,
    pub ts: String,
}

/// What a memo read answers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoData {
    Names(Vec<String>),
    Content(String),
    History(Vec<MemoEntry>),
}

/// Request: `{session?, op, scope, name?}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Ask {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<i64>,
    pub op: MemoOp,
    pub scope: MemoScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Reply: `{names}` / `{content}` / `{history}`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    pub data: MemoData,
}
