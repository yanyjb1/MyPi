//! One frame on the wire: kind, name, payload. Requests and replies share the shape.

use super::error::ErrorCode;
use serde::{Deserialize, Serialize};

/// The five frame kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Register,
    Call,
    Post,
    Subscribe,
    Notice,
}

/// One line in either direction. `payload` is always an object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub kind: Kind,
    pub name: String,
    #[serde(default = "empty_object")]
    pub payload: serde_json::Value,
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

impl Frame {
    pub fn new(kind: Kind, name: &str, payload: serde_json::Value) -> Frame {
        Frame {
            kind,
            name: name.to_string(),
            payload,
        }
    }

    /// A reply to `request`: same kind, same name, new payload.
    pub fn reply_to(request: &Frame, payload: serde_json::Value) -> Frame {
        Frame::new(request.kind, &request.name, payload)
    }

    /// An error frame answering `request`.
    pub fn error_to(request: &Frame, code: ErrorCode, message: &str) -> Frame {
        Frame::reply_to(request, crate::ipc::contract::error::payload(code, message))
    }
}
