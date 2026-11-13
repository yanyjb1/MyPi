//! One request's message list: the system prompt first, then the rebuilt history.

use std::sync::Arc;

use anyhow::Result;

use super::message::{Message, WireMessage};
use super::protocol::ProtocolRequest;
use super::transcript;

/// One lap's request body.
pub fn build(
    model: &str,
    max_output_tokens: u32,
    stream: bool,
    system: &str,
    stored: &[Message],
    tools_json: Arc<serde_json::Value>,
) -> Result<ProtocolRequest> {
    let mut messages = Vec::with_capacity(stored.len() + 1);
    if !system.trim().is_empty() {
        messages.push(WireMessage::System {
            content: system.to_string(),
        });
    }
    messages.extend(transcript::to_wire(stored));
    Ok(ProtocolRequest {
        model: model.to_string(),
        max_output_tokens,
        stream,
        messages,
        tools_json,
    })
}

/// A compaction's request body: the region being replaced, then the instruction.
pub fn compaction(
    model: &str,
    max_output_tokens: u32,
    stream: bool,
    system: &str,
    region: &[Message],
    instruction: &str,
    tools_json: Arc<serde_json::Value>,
) -> Result<ProtocolRequest> {
    let mut messages = Vec::with_capacity(region.len() + 2);
    if !system.trim().is_empty() {
        messages.push(WireMessage::System {
            content: system.to_string(),
        });
    }
    messages.extend(transcript::to_wire(region));
    messages.push(WireMessage::User {
        content: instruction.to_string().into(),
    });
    Ok(ProtocolRequest {
        model: model.to_string(),
        max_output_tokens,
        stream,
        messages,
        tools_json,
    })
}
