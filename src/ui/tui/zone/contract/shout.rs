//! The horn: one event a sub-zone (or the main zone) puts out for the others.

use serde_json::Value;

/// One event on the bus. The name is the whole routing decision; the content is
/// whatever the shouter's counterpart expects, and nobody else looks at it.
#[derive(Debug, Clone, PartialEq)]
pub struct Shout {
    pub name: &'static str,
    pub content: Value,
}

impl Shout {
    pub fn new(name: &'static str, content: Value) -> Shout {
        Shout { name, content }
    }
}

/// The names this front end shouts today.
pub mod name {
    /// The main zone: the terminal changed. `{cols, rows}`.
    pub const RESIZE: &str = "resize";
    /// The main zone: the conversation's working directory. `{path, temporary}`.
    pub const CWD: &str = "cwd";
    /// History: the last request's context size. `{used}`.
    pub const USAGE: &str = "usage";
    /// A turn opened or closed. `{on}`.
    pub const BUSY: &str = "busy";
    /// A rewind put a message back: the editor takes it. `{text}`.
    pub const PREFILL: &str = "prefill";
}
