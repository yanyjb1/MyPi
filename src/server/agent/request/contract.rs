//! Every shape that crosses into the loop. The loop fills what it owns; request reads it.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use anyhow::Result;

use super::live::Event;
use super::message::Message;
pub use super::message::Usage;

/// Where one request goes: which provider serves, which model answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transport {
    pub provider: String,
    pub model: String,
}

impl Transport {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

/// Why a turn stopped. Request answers the first two; the loop fills the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Model,
    User,
    Broken,
    Killed,
    Fence,
}

impl Stop {
    /// The `turns.stop_reason` spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Stop::Model => "stop",
            Stop::User => "interrupted",
            Stop::Broken => "broken",
            Stop::Killed => "killed",
            Stop::Fence => "fence",
        }
    }
}

/// How much of the history a compaction keeps verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// A budget in characters.
    Chars(usize),
    /// A fixed number of newest blocks.
    Blocks(usize),
}

/// One lap's input.
pub struct LapInput {
    pub session: i64,
    pub history: Arc<dyn History>,
    pub blocks: Arc<dyn BlockSink>,
    pub transport: Transport,
    pub system: String,
    pub tools: Vec<serde_json::Value>,
    pub stop: Arc<AtomicBool>,
    pub live: Sender<Event>,
}

/// One lap's answer.
#[derive(Debug)]
pub struct LapReply {
    pub stop: Stop,
    pub usage: Usage,
    pub calls: Vec<ToolCall>,
}

/// One call the model asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub args: String,
    pub intent: Option<String>,
}

/// One finished call, as the loop hands it back.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub call_id: String,
    pub ok: bool,
    pub result: String,
    pub memo: Option<String>,
    pub spill: Option<Spill>,
    pub duration_ms: u64,
}

/// The full text of an oversized tool result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spill {
    pub name: String,
    pub content: String,
}

/// What a block holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Assistant,
    Reasoning,
    ToolRequest,
    ToolResult,
}

/// One block, as request decides it. The loop turns it into a row.
#[derive(Debug, Clone, Default)]
pub struct Block {
    pub kind: Kind,
    /// When it landed.
    pub ts: Option<String>,
    /// Body text of a text or thinking block.
    pub content: Option<String>,
    /// The reply's bill; the last block of one reply carries it.
    pub usage: Option<Usage>,
    pub call_id: Option<String>,
    pub name: Option<String>,
    /// The arguments verbatim.
    pub args: Option<String>,
    /// The one line the model wrote about what it is doing.
    pub intent: Option<String>,
    /// Is this the batch's first call?
    pub first: Option<bool>,
    pub ok: Option<bool>,
    pub result: Option<String>,
    /// Extra material for a front end; never enters a request.
    pub memo: Option<String>,
    pub duration_ms: Option<u64>,
}

/// One spilled output and the citation it goes by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub name: String,
    pub content: String,
    pub hash: String,
    pub reference: String,
}

/// The write end of the block path.
pub trait BlockSink: Send + Sync {
    fn land(&self, block: Block, artifact: Option<Artifact>) -> Result<()>;
}

/// One stored block read back: its id and the message it carries.
#[derive(Debug, Clone)]
pub struct Piece {
    pub block: i64,
    pub message: Message,
}

/// One turn as the history reads it.
#[derive(Debug, Clone)]
pub struct TurnView {
    /// The block id ranges this turn holds, in order. A turn has several when other sessions
    /// wrote between its batches.
    pub ranges: Vec<[i64; 2]>,
    pub pieces: Vec<Piece>,
}

/// One citation already in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub hash: String,
    pub reference: String,
}

/// The read end of the history. It is bound to one thread: a storage reader is not shareable.
pub trait History {
    /// From the newest compaction marker on, in list order.
    fn turns(&self, session: i64) -> Result<Vec<TurnView>>;
    /// The hash that already holds this exact citation, if any.
    fn holder(&self, reference: &str) -> Result<Option<String>>;
}

/// One compaction request.
pub struct CompactInput {
    pub session: i64,
    pub history: Arc<dyn History>,
    pub transport: Transport,
    pub system: String,
    pub tools: Vec<serde_json::Value>,
    /// How much to keep verbatim; `None` takes the configured default.
    pub retention: Option<Retention>,
    /// What the summary should stress.
    pub focus: Option<String>,
    /// The instruction template; `{{focus}}` marks where the focus goes.
    pub instruction: String,
    pub stop: Arc<AtomicBool>,
    pub live: Sender<Event>,
}

/// What a compaction returns.
#[derive(Debug)]
pub struct CompactReply {
    pub text: String,
    pub usage: Usage,
    pub borrows: Vec<Borrow>,
}

/// One original turn's kept intervals, in block order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Borrow {
    pub ranges: Vec<[i64; 2]>,
}

/// One batch of finished calls, as the loop hands it back.
pub struct ResultsInput {
    pub session: i64,
    /// For the citations already in use: same content keeps its reference.
    pub history: Arc<dyn History>,
    /// Where an oversized output's full text is written.
    pub artifacts: PathBuf,
    pub calls: Vec<ToolCall>,
    pub results: Vec<ToolResult>,
}

/// The request half, as the loop sees it.
pub trait Ask: Send + Sync {
    fn lap(&self, input: LapInput) -> Result<LapReply>;
    /// Write one batch of results as blocks. A call without a result fails the whole batch.
    fn results(&self, input: ResultsInput, blocks: &dyn BlockSink) -> Result<()>;
    fn compact(&self, input: CompactInput) -> Result<CompactReply>;
}
