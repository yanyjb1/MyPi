//! The stream: the three requests that open one, and the eight payloads that travel it.

use super::{Block, NewBlock, SessionMeta};

/// `subscribe.watch` request: `{session}`.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct watch {
    pub session: i64,
}

/// `subscribe.submit` request.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct submit {
    pub session: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
}

/// `subscribe.compact` request.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct compact {
    pub session: i64,
    pub retention: Retention,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
}

/// How much of the transcript tail to keep verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Retention {
    Default,
    Tokens(usize),
    Blocks(usize),
}

/// How a round began.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartKind {
    Submit,
    Compact,
    Wake,
}

/// One round's usage on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct TurnUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
}

/// `subscribe.round_started` payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoundStarted {
    pub session: i64,
    pub turn: i64,
    pub kind: StartKind,
}

/// `subscribe.live` payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Live {
    pub session: i64,
    pub turn: i64,
    pub blocks: Vec<NewBlock>,
    pub done: bool,
}

/// `subscribe.blocks` payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Blocks {
    pub session: i64,
    pub turn: i64,
    pub blocks: Vec<Block>,
}

/// `subscribe.tool_output` payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolOutput {
    pub session: i64,
    pub turn: i64,
    pub call_id: String,
    pub text: String,
}

/// `subscribe.round_ended` payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoundEnded {
    pub session: i64,
    pub turn: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TurnUsage>,
}

/// `subscribe.meta` payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Meta {
    pub session: i64,
    pub meta: SessionMeta,
}

/// `subscribe.rewound` payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Rewound {
    pub session: i64,
    pub turn: i64,
    pub block: i64,
}

/// `subscribe.deleted` payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Deleted {
    pub session: i64,
}
