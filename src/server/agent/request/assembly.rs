//! One request's message list: the system prompt first, then the rebuilt history.

use anyhow::{Context, Result};

use super::message::{Message, ToolDef, WireMessage};
use super::protocol::ProtocolRequest;
use super::transcript;

/// One lap's request body.
#[allow(clippy::too_many_arguments)]
pub fn build(
    model: &str,
    max_output_tokens: u32,
    stream: bool,
    system: &str,
    stored: &[Message],
    manuals: &[serde_json::Value],
    session: i64,
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
        tools: manuals_to_tools(manuals)?,
        session_id: Some(session.to_string()),
    })
}

/// A compaction's request body: the region being replaced, then the instruction.
#[allow(clippy::too_many_arguments)]
pub fn compaction(
    model: &str,
    max_output_tokens: u32,
    stream: bool,
    system: &str,
    region: &[Message],
    instruction: &str,
    manuals: &[serde_json::Value],
    session: i64,
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
        tools: manuals_to_tools(manuals)?,
        session_id: Some(session.to_string()),
    })
}

fn manuals_to_tools(manuals: &[serde_json::Value]) -> Result<Vec<ToolDef>> {
    manuals
        .iter()
        .map(|manual| {
            serde_json::from_value(manual.clone()).context("a tool manual is not a function manual")
        })
        .collect()
}
