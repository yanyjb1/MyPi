//! The four error codes, and the error payload shape.

use serde::{Deserialize, Serialize};

/// Why a frame was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    UnknownName,
    BadPayload,
    Busy,
    Internal,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            ErrorCode::UnknownName => "unknown_name",
            ErrorCode::BadPayload => "bad_payload",
            ErrorCode::Busy => "busy",
            ErrorCode::Internal => "internal",
        };
        f.write_str(name)
    }
}

/// The payload of an error frame: `{"error": {"code", "message"}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub error: Inner,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inner {
    pub code: ErrorCode,
    pub message: String,
}

/// An error payload object for one frame.
pub fn payload(code: ErrorCode, message: &str) -> serde_json::Value {
    serde_json::to_value(ErrorPayload {
        error: Inner {
            code,
            message: message.to_string(),
        },
    })
    .unwrap_or_else(|_| serde_json::json!({}))
}
