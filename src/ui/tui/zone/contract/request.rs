//! What a zone asks the app to land on the session.

use serde_json::Value;

use super::kind::Kind;

/// A model address: the provider and the id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelId {
    pub provider: String,
    pub model: String,
}

/// One ask. The zone builds the content; the app fills in the session and sends it.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// One frame out. `answer` says whether the server replies to it.
    Send {
        kind: Kind,
        name: &'static str,
        content: Value,
        answer: bool,
    },
    /// Enter a conversation.
    Attach(i64),
    /// Leave the conversation.
    Detach,
    /// The structural window, as it stands.
    Structure,
}

impl Request {
    pub fn call(name: &'static str, content: Value) -> Request {
        Request::Send {
            kind: Kind::Call,
            name,
            content,
            answer: true,
        }
    }

    pub fn post(name: &'static str, content: Value) -> Request {
        Request::Send {
            kind: Kind::Post,
            name,
            content,
            answer: true,
        }
    }

    pub fn subscribe(name: &'static str, content: Value) -> Request {
        Request::Send {
            kind: Kind::Subscribe,
            name,
            content,
            answer: true,
        }
    }
}
