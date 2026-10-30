//! The stored message and the wire message: one read-side shape, one outbound shape.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The text a reply carries when the model said nothing at all. Three places must agree on
/// these bytes byte-for-byte, otherwise a restart changes what the model saw.
pub const DEFAULT_EMPTY_REPLY: &str = "SYSTEM: (no reply body — empty response)";

/// The text a reply carries when the user interrupted it.
pub const INTERRUPTED_REPLY: &str = "SYSTEM: UserInterrupted.";

/// One stored block, read back. Only what the wire needs; the rest of the columns stay behind.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    User {
        prefix: Option<String>,
        content: String,
        suffix: Option<String>,
    },
    Assistant {
        content: String,
    },
    Reasoning {
        content: String,
    },
    Call {
        call_id: String,
        name: String,
        args: String,
        first: bool,
    },
    Result {
        call_id: String,
        result: String,
    },
}

// ---- the outbound shapes ----

/// One conversation message; the role is its identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum WireMessage {
    System {
        content: String,
    },
    User {
        content: UserContent,
    },
    Assistant {
        #[serde(default, serialize_with = "serialize_content")]
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<WireCall>,
        /// Never serialized here: a protocol decides per host whether it goes out.
        #[serde(skip)]
        reasoning: Option<String>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

/// A user message: text, or text plus images.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UserContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl UserContent {
    pub fn text(&self) -> String {
        match self {
            Self::Text(t) => t.clone(),
            Self::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    ContentPart::ImageUrl { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

impl From<String> for UserContent {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

impl From<&str> for UserContent {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}

/// One part of a multimodal user message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One call on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireCall {
    pub id: String,
    #[serde(rename = "type", default = "function_kind")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// Raw JSON text: a `Value` round-trip would reorder keys and cost the prefix cache.
    pub arguments: String,
}

impl WireCall {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: function_kind(),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }

    pub fn name(&self) -> &str {
        &self.function.name
    }

    pub fn arguments_json(&self) -> serde_json::Result<Value> {
        serde_json::from_str(&self.function.arguments)
    }

    /// The model's own "what I am about to do"; empty when it wrote none.
    pub fn intent(&self) -> Option<String> {
        self.arguments_json()
            .ok()
            .and_then(|v| v.get("intent").and_then(|i| i.as_str()).map(String::from))
    }
}

fn function_kind() -> String {
    "function".to_string()
}

fn serialize_content<S: serde::Serializer>(v: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(t) => s.serialize_some(t),
        None => s.serialize_none(),
    }
}

/// One tool's manual.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    #[serde(rename = "type", default = "function_kind")]
    pub kind: String,
    pub function: FunctionDef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolDef {
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self {
            kind: function_kind(),
            function: FunctionDef {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

/// One request's bill.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt: u64,
    pub completion: u64,
    pub cached: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, rhs: Self) {
        self.prompt += rhs.prompt;
        self.completion += rhs.completion;
        self.cached += rhs.cached;
        self.cache_write += rhs.cache_write;
        self.reasoning += rhs.reasoning;
    }
}

/// One complete reply, assembled from deltas.
#[derive(Debug, Clone, Default)]
pub struct AssistantMessage {
    pub content: String,
    pub calls: Vec<WireCall>,
    pub usage: Usage,
    pub reasoning: Option<String>,
    pub interrupted: bool,
}
