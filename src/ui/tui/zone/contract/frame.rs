//! One frame as a zone sees it: class, name, content. The app fills it in.

use serde_json::Value;

/// One frame crossing the zone boundary. The class is the wire's kind, unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub class: String,
    pub name: String,
    pub content: Value,
}

impl Frame {
    pub fn new(class: impl Into<String>, name: impl Into<String>, content: Value) -> Frame {
        Frame {
            class: class.into(),
            name: name.into(),
            content,
        }
    }

    /// A fact the front end itself produced.
    pub fn is_local(&self) -> bool {
        self.class == "local"
    }
}
