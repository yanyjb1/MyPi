//! What the loop asks the request half for.

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use anyhow::Result;

use crate::server::agent::r#loop::out::Event;
use crate::server::agent::r#loop::session::artifact::ArtifactStore;
use crate::server::agent::r#loop::session::transport::Transport;
use crate::server::agent::request::Block;
use crate::server::storage::Reader;

/// Why a turn stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Model,
    User,
    Broken,
    Killed,
    Fence,
}

impl Stop {
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

/// One request's usage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt: u64,
    pub completion: u64,
    pub cached: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, rhs: Self) {
        self.prompt += rhs.prompt;
        self.completion += rhs.completion;
        self.cached += rhs.cached;
        self.cache_write += rhs.cache_write;
        self.reasoning += rhs.reasoning;
    }
}

/// One call the model asked for.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub args: String,
    pub intent: Option<String>,
}

/// One oversized output the request cut out of a tool result.
#[derive(Debug, Clone)]
pub struct Artifact {
    pub name: String,
    pub content: String,
    pub hash: String,
    pub reference: String,
}

/// The write end of the block path. The request half owns what a block is;
/// the loop only turns it into storage rows.
pub trait BlockSink: Send + Sync {
    fn block(&self, block: Block, artifact: Option<Artifact>) -> Result<()>;
}

/// One lap's input.
pub struct LapInput {
    pub session: i64,
    pub turn: i64,
    pub reader: Reader,
    pub transport: Transport,
    pub system: String,
    pub tools: Vec<serde_json::Value>,
    pub max_output_tokens: Option<u32>,
    pub stop: std::sync::Arc<AtomicBool>,
    pub out: Sender<Event>,
    pub blocks: std::sync::Arc<dyn BlockSink>,
    pub artifacts: std::sync::Arc<ArtifactStore>,
    pub first: bool,
}

/// One lap's answer.
pub struct LapReply {
    pub stop: Stop,
    pub usage: Usage,
    pub calls: Vec<ToolCall>,
}

/// One compaction request.
pub struct CompactInput {
    pub session: i64,
    pub turn: i64,
    pub reader: Reader,
    pub transport: Transport,
    pub system: String,
    pub tools: Vec<serde_json::Value>,
    pub retention: Option<super::super::session::book::Retention>,
    pub focus: Option<String>,
    pub stop: std::sync::Arc<AtomicBool>,
    pub out: Sender<Event>,
    pub blocks: std::sync::Arc<dyn BlockSink>,
    pub artifacts: std::sync::Arc<ArtifactStore>,
}

/// One old turn's retained intervals, in list order.
#[derive(Debug, Clone)]
pub struct Borrow {
    pub ranges: Vec<[i64; 2]>,
}

/// What a compaction returns.
pub struct CompactReply {
    pub text: String,
    pub usage: Usage,
    pub borrows: Vec<Borrow>,
}

/// The request half, as the loop sees it.
pub trait Ask: Send + Sync {
    fn lap(&self, input: LapInput) -> Result<LapReply>;
    fn hand_results(
        &self,
        session: i64,
        turn: i64,
        calls: &[ToolCall],
        results: &[super::batch::ToolResult],
        blocks: &dyn BlockSink,
    ) -> Result<()>;
    fn compact(&self, input: CompactInput) -> Result<CompactReply>;
}
