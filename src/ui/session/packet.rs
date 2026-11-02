//! The normalized unit: one frame's three parts, forwarded unchanged.

use serde_json::Value;

/// One frame as it crosses the session: `kind`, `name`, `content`.
///
/// The session never reads `content` except where its two caches must be kept (a block's id,
/// a turn's ranges, the session a push belongs to).
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    pub kind: String,
    pub name: String,
    pub content: Value,
}

impl Packet {
    pub fn new(kind: impl Into<String>, name: impl Into<String>, content: Value) -> Packet {
        Packet {
            kind: kind.into(),
            name: name.into(),
            content,
        }
    }

    /// A fact the session itself produced.
    pub fn local(name: impl Into<String>, content: Value) -> Packet {
        Packet::new("local", name, content)
    }
}
