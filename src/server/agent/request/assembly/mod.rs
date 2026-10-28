pub mod compact;
pub mod history;

use crate::server::agent::request::message::{Message, ToolDef, WireMessage};
use crate::server::agent::request::protocols::ProtocolRequest;

pub(crate) fn with_system(messages: Vec<WireMessage>, system: &str) -> Vec<WireMessage> {
    if system.trim().is_empty() {
        return messages;
    }
    let mut out = Vec::with_capacity(messages.len() + 1);
    out.push(WireMessage::System {
        content: system.to_string(),
    });
    out.extend(messages);
    out
}

pub(crate) fn build(
    model: &str,
    max_output_tokens: u32,
    stream: bool,
    system: &str,
    stored: &[Message],
    tools: &[ToolDef],
    session_id: Option<&str>,
) -> ProtocolRequest {
    let wire = history::to_wire(stored, &mut history::Rebuild::default());
    ProtocolRequest {
        model: model.to_string(),
        max_output_tokens,
        stream,
        messages: with_system(wire, system),
        tools: tools.to_vec(),
        session_id: session_id.map(str::to_string),
    }
}
