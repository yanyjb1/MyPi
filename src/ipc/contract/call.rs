//! Call: one question, one answer, read-only.

use std::path::PathBuf;

use crate::commands::Candidate;

use super::{Block, SessionMeta, TodoPhase};

/// A one-shot question.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Call {
    Session { session: i64, ask: SessionAsk },
    Tool(ToolAsk),
    Server(ServerAsk),
}

/// What a [`Call`] answers with.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Reply {
    Session(SessionReply),
    Tool(ToolReply),
    Server(ServerReply),
}

/// Answers to the session group.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SessionReply {
    Init(InitReply),
    Blocks(Vec<Block>),
    Meta(SessionState),
    Usage(UsageReply),
}

/// Answers to the tool group.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ToolReply {
    Todo(Vec<TodoPhase>),
    Memo(MemoData),
    Children(Vec<ChildRow>),
}

/// Answers to the server group.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ServerReply {
    Config(ConfigValue),
    Pools(CandidatePools),
    Sessions(Vec<SessionRow>),
}

/// Questions about one session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SessionAsk {
    Init { limit: usize },
    Blocks { ranges: Vec<[i64; 2]> },
    Meta,
    Usage { lang: UsageLang },
}

/// The session's first frame.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InitReply {
    pub structure: Vec<TurnShape>,
    pub blocks: Vec<Block>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo: Option<Vec<TodoPhase>>,
}

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

/// One session as it stands.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionState {
    pub meta: SessionMeta,
    pub cwd: PathBuf,
}

/// Number wording for the usage panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageLang {
    #[default]
    En,
    Zh,
}

/// The bill for one session.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UsageReply {
    pub lang: UsageLang,
    pub groups: Vec<ModelUsage>,
}

/// One usage row: one model's cumulative usage and bill within a session.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelUsage {
    pub id: String,
    pub provider: Option<String>,
    pub rounds: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost: Option<UsageCost>,
    pub currency: Option<String>,
}

/// The four billed cells.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct UsageCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Questions about tools.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ToolAsk {
    Todo {
        session: i64,
    },
    Memo {
        session: Option<i64>,
        op: MemoOp,
        scope: MemoScope,
        name: Option<String>,
    },
    Children {
        session: i64,
    },
}

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

/// What a memo read answers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MemoData {
    Names(Vec<String>),
    Content(String),
    History(Vec<MemoEntry>),
}

/// One stored version of a memo.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemoEntry {
    pub id: i64,
    pub content: String,
    pub ts: String,
}

/// One child session.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChildRow {
    pub anchor: i64,
    pub session: SessionRow,
    pub intent: Option<String>,
    pub state: ChildState,
}

/// Where a child stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildState {
    Queued,
    Running,
    Ended,
}

/// Questions about the server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ServerAsk {
    Config { key: String },
    Pools,
    Sessions { under: Option<PathBuf> },
}

/// One config value.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<ConfigValue>),
}

/// The two completion pools.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CandidatePools {
    pub models: Vec<Candidate>,
    pub profiles: Vec<Candidate>,
}

/// One picker row.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionRow {
    pub meta: SessionMeta,
    pub first_user: Option<String>,
    pub bytes: i64,
    pub last_turn: Option<i64>,
}
