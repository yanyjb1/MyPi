//! The tool input one turn gets.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use super::ask::ToolCall;
use super::batch::BatchInput;
use crate::server::agent::request::live::Event;
use crate::server::agent::tools::contract::call::Cwd;

/// Build the per-turn tool input. The tool layer owns the concrete `Env`.
#[allow(clippy::too_many_arguments)]
pub fn build(
    session: i64,
    turn: i64,
    root: PathBuf,
    cwd: Cwd,
    tools: Vec<String>,
    calls: Vec<ToolCall>,
    stop: std::sync::Arc<AtomicBool>,
    out: Sender<Event>,
) -> BatchInput {
    BatchInput {
        session,
        turn,
        root,
        cwd,
        tools,
        calls,
        stop,
        out,
    }
}
