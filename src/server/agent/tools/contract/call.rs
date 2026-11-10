//! One batch in, one result per call out: the loop's entry and the working directory it shares.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::mpsc::Sender;

use anyhow::Result;

use super::output::ToolResult;
use crate::server::agent::request::contract::ToolCall;
use crate::server::agent::request::live::Event;

/// The session's working directory handle: `cd` writes it, the next batch reads it.
pub type Cwd = Arc<RwLock<PathBuf>>;

/// One batch's input.
pub struct BatchInput {
    pub session: i64,
    /// The session's start directory; it never moves.
    pub root: PathBuf,
    pub cwd: Cwd,
    /// The session's tool names; the one list the profile gate reads.
    pub tools: Vec<String>,
    /// This batch, in execution order.
    pub calls: Vec<ToolCall>,
    pub stop: Arc<crate::server::platform::stop::Stop>,
    pub out: Sender<Event>,
}

/// The tool layer, as the loop sees it.
pub trait Batch: Send + Sync {
    fn run(&self, input: BatchInput) -> Result<Vec<ToolResult>>;
}
