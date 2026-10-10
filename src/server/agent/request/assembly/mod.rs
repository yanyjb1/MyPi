//! Assembly: everything outgoing grows here — `history` (flattening), `body` (the one construction
//! point), `compat` (per-host), `compact` (a prepared request).

pub mod body;
pub mod compact;
pub mod compat;
pub mod history;

use crate::server::agent::request::assembly::compat::Resolved;
use crate::server::agent::request::message::{Message, ToolDef, WireMessage};

/// Where the system prompt sits: **first**, in every request we build, and known in this one place.
pub(crate) fn with_system(messages: Vec<WireMessage>, system: &str) -> Vec<WireMessage> {
    let mut messages = messages;
    messages.insert(
        0,
        WireMessage::System {
            content: system.to_string(),
        },
    );
    messages
}

/// Assemble one outgoing request: messages → wire array → body.
pub(crate) fn build(
    model: &str,
    max_tokens: u32,
    stream: bool,
    system: &str,
    stored: &[Message],
    tools: &[ToolDef],
    compat: &Resolved,
    session_id: Option<&str>,
) -> serde_json::Value {
    let wire = history::to_wire(stored, &mut history::Rebuild::default());
    let messages = body::messages_to_wire(&with_system(wire, system), compat);
    let tools = serde_json::to_value(tools).expect("tool serialization is infallible");
    body::body_json(model, max_tokens, stream, &messages, &tools, compat, session_id)
}
