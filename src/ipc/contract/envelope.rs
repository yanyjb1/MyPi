//! The envelope: registration, one question, one answer, and the error code.

use super::{Event, Opened, Post, PostDone, Reply, Subscribe};

/// Bumped when a wire shape changes; a mismatched front end is refused.
pub const PROTOCOL_VERSION: u16 = 1;

/// A front end's own id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Viewer(pub String);

/// Why a door failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NoHello,
    BadRequest,
    NotFound,
    Busy,
    UnknownKey,
    Internal,
}

/// One line from a front end.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Ask {
    Hello { viewer: Viewer, protocol: u16 },
    Bye,
    Call { call: super::call::Call },
    Post { post: Post },
    Subscribe { subscribe: Subscribe },
}

/// One line from the server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Answer {
    Reply { reply: Reply },
    Done { done: PostDone },
    Opened { opened: Opened },
    Stream { event: Event },
    Error { code: ErrorCode, message: String },
}

impl Answer {
    pub fn error(code: ErrorCode, e: &anyhow::Error) -> Answer {
        Answer::Error {
            code,
            message: format!("{e:#}"),
        }
    }
}
