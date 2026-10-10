//! One request's input, output, delta outlet and metadata. `RequestContent` carries the session id
//! only; `AnswerSink` hands out the two streams, where `false` means stop.

use crate::server::agent::request::message::{AssistantMessage, StopReason, ToolDef, Usage};

/// One request's input.
pub struct RequestContent {
    /// The session id: history and filing both need it.
    pub session: i64,
    /// The system prompt in force.
    pub system: String,
    /// The tool manual.
    pub tools: Vec<ToolDef>,
    /// The output cap for this turn.
    pub max_tokens: u32,
}

/// One request's output.
pub struct Answer {
    /// The decoded reply.
    pub message: AssistantMessage,
    /// How it stopped.
    pub stop: StopReason,
    /// Tokens used.
    pub usage: Usage,
    /// The request metadata, handed to storage.
    pub meta: RequestMeta,
}

/// Where the deltas go. `false` = stop.
pub trait AnswerSink {
    /// Text delta.
    fn content(&mut self, text: &str) -> bool;
    /// Reasoning delta.
    fn reasoning(&mut self, text: &str) -> bool;
}

/// A request's metadata — the part of one round that does **not** change with the conversation. The
/// shape belongs to storage; this only passes it out.
pub use crate::server::storage::RequestMeta;
