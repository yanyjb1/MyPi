//! Every shape that crosses into the loop. The loop fills what it owns; request reads it.

use std::sync::Arc;
use std::sync::mpsc::Sender;

use anyhow::Result;

use super::live::Event;
use super::message::Message;
pub use super::message::Usage;
use super::transport::worker::SessionWorker;

/// Where one request goes: which provider serves, which model answers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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
    /// A tool asked for the round to end, `final submit` being the one that does.
    Finished,
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
            Stop::Finished => "finished",
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
    /// The session's manuals, serialized once at booking; the wire body takes a copy.
    pub tools_json: Arc<serde_json::Value>,
    pub stop: Arc<crate::server::platform::stop::Stop>,
    /// This session's worker: the child that owns its sockets.
    pub worker: Arc<SessionWorker>,
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

/// What one item of a call answered: its own outcome, and its failure text when it failed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ItemOk {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ItemOk {
    pub fn ok() -> Self {
        Self {
            ok: true,
            error: None,
        }
    }

    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(error.into()),
        }
    }
}

/// One finished call, as the loop hands it back.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub call_id: String,
    /// One entry per item the call's arguments named.
    pub items: Vec<ItemOk>,
    pub result: String,
    /// Why the call failed, when it failed by being cut short.
    pub reason: Option<String>,
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
    /// One outcome per item the call's arguments named.
    pub ok: Option<Vec<ItemOk>>,
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
    /// The round's own id.
    pub turn: i64,
    pub pieces: Vec<Piece>,
    /// This round wrote the history a compaction replaced.
    pub compacted: bool,
}

impl TurnView {
    /// What this round adds to a request. A compaction round adds only its summary, and the
    /// summary takes the user role: the request after a compaction opens with it. The
    /// instruction that produced it stays in storage as the record of that compaction.
    pub fn sent(&self) -> Vec<Piece> {
        if !self.compacted {
            return self.pieces.clone();
        }
        self.pieces
            .iter()
            .filter_map(|piece| match &piece.message {
                Message::Assistant { content } => Some(Piece {
                    block: piece.block,
                    message: Message::User {
                        prefix: None,
                        content: content.clone(),
                        suffix: None,
                    },
                }),
                _ => None,
            })
            .collect()
    }
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
    /// Every message the list carries, shared; a lap never copies them.
    fn messages(&self, session: i64) -> Result<Arc<Vec<Message>>> {
        Ok(Arc::new(
            self.turns(session)?
                .into_iter()
                .flat_map(|turn| turn.pieces.into_iter().map(|piece| piece.message))
                .collect(),
        ))
    }
    /// The hash that already holds this exact citation, if any.
    fn holder(&self, reference: &str) -> Result<Option<String>>;
}

/// One compaction request.
pub struct CompactInput {
    pub session: i64,
    /// The round writing this compaction; its own blocks are not history yet.
    pub turn: i64,
    pub history: Arc<dyn History>,
    pub transport: Transport,
    pub system: String,
    /// The session's manuals, serialized once at booking.
    pub tools_json: Arc<serde_json::Value>,
    /// How much to keep verbatim; `None` takes the configured default.
    pub retention: Option<Retention>,
    /// The instruction this compaction sends, rendered once when the round opened.
    pub instruction: String,
    pub stop: Arc<crate::server::platform::stop::Stop>,
    /// This session's worker: the child that owns its sockets.
    pub worker: Arc<SessionWorker>,
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
