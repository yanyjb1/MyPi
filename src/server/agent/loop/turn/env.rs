//! The tool environment one turn gets.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use super::ask::ToolCall;
use super::batch::BatchInput;
use crate::server::agent::r#loop::session::artifact::ArtifactStore;
use crate::server::agent::request::live::Event;

/// Build the per-turn tool input. The tool layer owns the concrete `Env`.
pub fn build(
    session: i64,
    turn: i64,
    cwd: PathBuf,
    calls: Vec<ToolCall>,
    stop: std::sync::Arc<AtomicBool>,
    out: Sender<Event>,
    artifacts: std::sync::Arc<ArtifactStore>,
) -> BatchInput {
    BatchInput {
        session,
        turn,
        cwd,
        calls,
        stop,
        out,
        artifacts,
    }
}
