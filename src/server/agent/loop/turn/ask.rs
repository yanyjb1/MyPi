//! What the loop asks the request half for. The shapes live with the request half.

pub use crate::server::agent::request::contract::{
    Artifact, Ask, BlockSink, Borrow, CompactInput, CompactReply, LapInput, LapReply, ResultsInput,
    Stop, ToolCall, Usage,
};
