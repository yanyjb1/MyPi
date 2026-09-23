//! Event types — background thread -> main thread.

/// Events the background thread sends the main thread during a turn.
pub enum AppEvent {
    /// Streaming text delta.
    Delta(String),
    /// Streaming reasoning delta.
    ReasoningDelta(String),
    /// A turn finished (usage for billing, stop_reason to distinguish interrupts).
    TurnDone(crate::ai::types::Usage, crate::ai::types::StopReason),
    /// A tool started executing.
    ToolStart { call_id: String, name: String, args_summary: String },
    /// A tool finished executing.
    ToolFinish {
        call_id: String,
        name: String,
        ok: bool,
        result: String,
    },
    /// An error occurred.
    Error(String),
    /// Turn finalized: the whole round's entries ship to the main thread for one-shot persistence.
    Commit(Vec<crate::tui::components::chat::Entry>),
    /// The background thread is exiting.
    Done,
}
