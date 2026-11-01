//! The exit vocabulary: everything a front end receives, plus the shapes it carries.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use super::structure::Turn;

/// What kind of request failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    Read,
    Change,
    Round,
    Watch,
}

/// One event, in arrival order.
#[derive(Debug)]
pub enum Event {
    // pushes
    Live(Piece),
    Notice {
        level: NoticeLevel,
        text: String,
    },
    Structure(Turn),
    Change(Change),

    // answers
    Opened {
        turn: i64,
    },
    Init {
        structure: Vec<Turn>,
        blocks: Vec<Block>,
        todo: Option<Vec<TodoItem>>,
    },
    Blocks(Vec<Block>),
    Meta(SessionMeta),
    Usage(UsageReply),
    Todo(Vec<TodoItem>),
    Memo(MemoData),
    Children(Vec<ChildRow>),
    Config {
        key: String,
        value: ConfigValue,
    },
    Pools(Pools),
    Sessions(Vec<SessionMeta>),

    // write outcomes
    Created(i64),
    Done,
    Busy,
    Cut(Cut),
    DefaultModel(String),

    Failed {
        kind: FailKind,
        message: String,
    },
}

/// One piece of the reply as it arrives, or the end of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Thinking(String),
    Arguments {
        call_id: String,
        text: String,
    },
    Boundary,
    ToolStart {
        call_id: String,
        name: String,
        args: String,
        intent: Option<String>,
    },
    ToolProgress {
        call_id: String,
        text: String,
    },
    ToolEnd {
        call_id: String,
        ok: bool,
        result: String,
        memo: Option<String>,
        duration_ms: u64,
    },
}

/// Something said to whoever is watching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warn,
    Error,
}

/// A change another front end made to the session.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Meta(SessionMeta),
    Rewound { turn: i64, block: i64, cut: Cut },
    Deleted,
}

/// What survived a rewind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cut {
    Boundary { turn: i64 },
    Rescued { turn: i64 },
    Retyped { turn: Option<i64> },
}

/// What a block is (the `type` column, typed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    User,
    Wake,
    Assistant,
    Reasoning,
    ToolRequest,
    ToolResult,
}

impl BlockKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BlockKind::User => "user",
            BlockKind::Wake => "wake",
            BlockKind::Assistant => "assistant",
            BlockKind::Reasoning => "reasoning",
            BlockKind::ToolRequest => "tool_request",
            BlockKind::ToolResult => "tool_result",
        }
    }
}

/// One landed block: the `blocks` row, decoded.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Block {
    pub id: i64,
    pub kind: BlockKind,
    pub content: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub usage_total: Option<u64>,
    pub usage_prompt: Option<u64>,
    pub usage_completion: Option<u64>,
    pub usage_cached: Option<u64>,
    pub usage_cache_write: Option<u64>,
    pub usage_reasoning: Option<u64>,
    pub call_id: Option<String>,
    pub name: Option<String>,
    pub args: Option<String>,
    pub intent: Option<String>,
    pub text: Option<String>,
    pub first: Option<bool>,
    pub ok: Option<bool>,
    pub result: Option<String>,
    pub memo: Option<String>,
    pub duration_ms: Option<i64>,
    pub ts: Option<String>,
}

/// A session row.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SessionMeta {
    pub id: i64,
    pub name: Option<String>,
    pub started_at: String,
    pub cwd: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub anchor: Option<i64>,
    pub meta: Option<i64>,
}

/// One task-list entry.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

/// A child session's row.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChildRow {
    pub id: i64,
    pub anchor: i64,
}

/// Session totals and one row per `provider:model`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UsageReply {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub cache_hit_rate: f64,
    pub groups: Vec<UsageGroup>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UsageGroup {
    pub address: String,
    pub provider: String,
    pub model: String,
    pub rounds: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub cache_write_tokens: u64,
    pub share: f64,
    pub price: Option<Price>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Price {
    pub currency: String,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
    pub cny: f64,
    pub usd: f64,
}

/// A memo answer: names, one body, or its versions.
#[derive(Debug, Clone, PartialEq)]
pub enum MemoData {
    Names(Vec<String>),
    Content(String),
    History(Vec<MemoVersion>),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MemoVersion {
    pub id: i64,
    pub content: String,
    pub ts: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum MemoReply {
    Names { names: Vec<String> },
    Content { content: String },
    History { history: Vec<MemoVersion> },
}

impl MemoReply {
    pub fn into_data(self) -> MemoData {
        match self {
            MemoReply::Names { names } => MemoData::Names(names),
            MemoReply::Content { content } => MemoData::Content(content),
            MemoReply::History { history } => MemoData::History(history),
        }
    }
}

/// The models and profiles a session may be opened with.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Pools {
    pub models: Vec<ModelEntry>,
    pub profiles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ModelEntry {
    pub provider: String,
    pub id: String,
    pub address: String,
    pub name: String,
    pub context_window: u64,
    pub max_output_tokens: u64,
    pub currency: String,
}

/// One configuration value, typed.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<ConfigValue>),
    Object(BTreeMap<String, ConfigValue>),
}

impl ConfigValue {
    pub fn from_json(value: Value) -> ConfigValue {
        match value {
            Value::Null => ConfigValue::Null,
            Value::Bool(on) => ConfigValue::Bool(on),
            Value::Number(number) => match number.as_i64() {
                Some(number) => ConfigValue::Int(number),
                None => ConfigValue::Float(number.as_f64().unwrap_or(0.0)),
            },
            Value::String(text) => ConfigValue::Text(text),
            Value::Array(items) => {
                ConfigValue::List(items.into_iter().map(ConfigValue::from_json).collect())
            }
            Value::Object(map) => ConfigValue::Object(
                map.into_iter()
                    .map(|(key, value)| (key, ConfigValue::from_json(value)))
                    .collect(),
            ),
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            ConfigValue::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            ConfigValue::Int(number) => Some(*number),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ConfigValue::Bool(on) => Some(*on),
            _ => None,
        }
    }
}

/// A model address: the provider and the id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelId {
    pub provider: String,
    pub model: String,
}

/// How much of the history a compaction keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    Chars(usize),
    Blocks(usize),
}

/// What a memo read asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoOp {
    List,
    Show,
    History,
}

impl MemoOp {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoOp::List => "list",
            MemoOp::Show => "show",
            MemoOp::History => "history",
        }
    }
}

/// Which memos a read covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoScope {
    Global,
    Current,
}

impl MemoScope {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoScope::Global => "global",
            MemoScope::Current => "current",
        }
    }
}

/// The answer to `call_init`.
#[derive(Debug, Deserialize)]
pub struct InitReply {
    pub structure: Vec<TurnShape>,
    pub blocks: Vec<Block>,
    pub todo: Option<Vec<TodoItem>>,
}

/// One round's wire shape: its id and its block id pairs.
#[derive(Debug, Deserialize)]
pub struct TurnShape {
    pub turn: i64,
    pub ranges: Vec<[i64; 2]>,
}

#[derive(Debug, Deserialize)]
pub struct BlocksReply {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Deserialize)]
pub struct CreatedReply {
    pub session: i64,
}

#[derive(Debug, Deserialize)]
pub struct OpenedReply {
    pub turn: i64,
}

#[derive(Debug, Deserialize)]
pub struct CutReply {
    pub cut: Cut,
}

#[derive(Debug, Deserialize)]
pub struct DefaultModelReply {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct ChildrenReply {
    pub rows: Vec<ChildRow>,
}

#[derive(Debug, Deserialize)]
pub struct SessionsReply {
    pub rows: Vec<SessionMeta>,
}
