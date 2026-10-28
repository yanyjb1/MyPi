use crate::server::agent::request::message::{AssistantMessage, StopReason, ToolDef, Usage};
use crate::server::agent::request::retry::RetryNotice;
use crate::server::storage::NewBlock;

/// One request's input.
pub struct RequestContent {
    pub session: i64,
    pub system: String,
    pub tools: Vec<ToolDef>,
    pub max_output_tokens: u32,
    pub profile: String,
}

/// One request's output.
pub struct Answer {
    pub message: AssistantMessage,
    pub stop: StopReason,
    pub usage: Usage,
    pub meta: RequestMeta,
}

/// Receives one decoded block piece; `false` stops the stream.
pub trait BlockSink {
    fn block(&mut self, block: NewBlock, done: bool) -> bool;
}

/// Receives retry notices.
pub trait RetrySink {
    fn retry(&mut self, notice: &RetryNotice);
}

pub use crate::server::storage::RequestMeta;
