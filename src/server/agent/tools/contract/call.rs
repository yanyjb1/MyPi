//! One batch in, one result per call out: the tool layer's entry and the directory it shares.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use anyhow::Result;

use super::env::Env;
use super::output::ToolResult;
use crate::server::agent::request::contract::ToolCall;
use crate::server::agent::request::live::Event;

/// The session's working directory handle: `cd` writes it, every call reads it.
pub type Cwd = Arc<RwLock<PathBuf>>;

/// The tool layer, as the loop sees it.
pub trait Batch: Send + Sync {
    fn run(&self, env: &mut Env, calls: &[ToolCall], out: Sender<Event>)
    -> Result<Vec<ToolResult>>;
}
