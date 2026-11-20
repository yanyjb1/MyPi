//! One frame as a zone sees it: kind, name, content. The app fills it in.

use serde_json::Value;

/// One frame crossing the zone boundary: the wire's kind, unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub kind: String,
    pub name: String,
    pub content: Value,
}

impl Frame {
    pub fn new(kind: impl Into<String>, name: impl Into<String>, content: Value) -> Frame {
        Frame {
            kind: kind.into(),
            name: name.into(),
            content,
        }
    }

    /// A fact the front end itself produced.
    pub fn is_local(&self) -> bool {
        self.kind == "local"
    }
}
