// The payload of an error frame: one machine-keyed text, or one upstream body.

use serde::{Deserialize, Serialize};

use super::text::Text;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub error: Text,
}

/// An error payload object for one frame.
pub fn payload(text: Text) -> serde_json::Value {
    serde_json::to_value(ErrorPayload { error: text }).unwrap_or_else(|_| serde_json::json!({}))
}
