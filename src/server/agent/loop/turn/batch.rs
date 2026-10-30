//! What the loop asks the tool layer for.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use anyhow::Result;

use super::ask::{Artifact, ToolCall};
use crate::server::agent::r#loop::out::Event;

/// One tool effect the loop turns into rows.
#[derive(Debug, Clone)]
pub enum Effect {
    Memo {
        anchor: i64,
        name: String,
        content: String,
        ts: String,
    },
    Alias {
        anchor: i64,
        name: String,
        command: String,
        ts: String,
    },
    Subagent {
        anchor: i64,
        provider: String,
        model: String,
        system: String,
        tools: Vec<serde_json::Value>,
        cwd: PathBuf,
        start: String,
    },
}

/// One finished call.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub call_id: String,
    pub ok: bool,
    pub result: String,
    pub memo: Option<String>,
    pub artifact: Option<Artifact>,
    pub duration_ms: u64,
    pub effects: Vec<Effect>,
    pub stop: bool,
}

/// One batch's input.
pub struct BatchInput {
    pub session: i64,
    pub turn: i64,
    pub cwd: PathBuf,
    pub calls: Vec<ToolCall>,
    pub stop: std::sync::Arc<AtomicBool>,
    pub out: Sender<Event>,
}

/// The tool layer, as the loop sees it.
pub trait Batch: Send + Sync {
    fn run(&self, input: BatchInput) -> Result<Vec<ToolResult>>;
}
