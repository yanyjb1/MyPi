//! Block constructors: the shapes every protocol hands the write end.

use super::super::contract::{Block, Kind};
use super::super::message::WireCall;

/// A text or thinking block.
pub fn text(kind: Kind, body: &str) -> Block {
    Block {
        kind,
        content: Some(body.to_string()),
        ..Block::default()
    }
}

/// A reply's body.
pub fn assistant(body: &str) -> Block {
    text(Kind::Assistant, body)
}

/// One call the model placed.
pub fn call(call: &WireCall, first: bool) -> Block {
    Block {
        kind: Kind::ToolRequest,
        call_id: Some(call.id.clone()),
        name: Some(call.name().to_string()),
        args: Some(call.function.arguments.clone()),
        intent: call.intent(),
        first: Some(first),
        ..Block::default()
    }
}
