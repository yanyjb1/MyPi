//! The two shape sets. `Message` is what is stored (five kinds, one block per message); `WireMessage`
//! is what goes out (`role`-tagged, OpenAI-compatible). `assembly` translates between them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use anyhow::{Context as _, Result};

use crate::server::storage::{Block, BlockKind, NewBlock};

/// Stand-in for an assistant reply that arrived empty. **Three writers agree on it byte-for-byte**: a
/// restart with different bytes would change what the model saw and cost the prefix cache.
pub const DEFAULT_EMPTY_REPLY: &str = "SYSTEM: (no reply body — empty response)";

/// The text persisted for a round the user **actually** interrupted. One constant, not config: it is
/// the system's voice, and it must stay distinct from [`DEFAULT_EMPTY_REPLY`].
pub const INTERRUPTED_REPLY: &str = "SYSTEM: UserInterrupted.";

/// One message: what a block holds. Five kinds, one per protocol message; the field names line up with
/// the block's columns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    /// One thing the user said. `prefix` / `suffix` are injected and frozen at write time; the body
    /// stays clean.
    User {
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        suffix: Option<String>,
    },
    /// The model's final reply; `usage` feeds the stats line.
    Assistant {
        content: String,
        usage: Option<UsageSummary>,
    },
    /// The thinking chain, its own message and block. It is carried into the rebuilt protocol message;
    /// whether it reaches the wire is the assembly layer's call.
    Reasoning { content: String },
    /// A tool call: `args` is the raw JSON the model sent, `call_id` pairs it. `text` and `first`
    /// preserve the wire shape, so a run of calls replays as one assistant message.
    ToolRequest {
        call_id: String,
        name: String,
        args: String,
        intent: String,
        text: String,
        first: bool,
    },
    /// A tool result: `result` is the exact text sent to the model, `memo` the structured payload for
    /// front ends. `duration_ms` is measured by the loop.
    ToolResult {
        call_id: String,
        name: String,
        ok: bool,
        result: String,
        #[serde(default)]
        memo: Option<Value>,
        #[serde(default)]
        duration_ms: u64,
    },
}

// Fields the stats line needs (the minimal set extracted from usage).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UsageSummary {
    pub total_tokens: u64,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
}

impl Message {
    /// Build a stats summary from the ai layer's usage.
    pub fn usage_summary(u: &Usage) -> UsageSummary {
        UsageSummary {
            total_tokens: u.total_tokens,
            prompt_tokens: u.prompt_tokens,
            cached_tokens: u.cached_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens,
            reasoning_tokens: u.reasoning_tokens.unwrap_or(0),
            cache_write_tokens: u.cache_write_tokens.unwrap_or(0),
        }
    }

    /// The row this message becomes. `ts` is when it lands: the caller reads the clock.
    pub fn to_block(&self, ts: Option<String>) -> NewBlock {
        let mut b = NewBlock {
            ts,
            ..NewBlock::default()
        };
        match self {
            Message::User {
                content,
                prefix,
                suffix,
            } => {
                b.kind = BlockKind::User;
                b.content = Some(content.clone());
                b.prefix = prefix.clone();
                b.suffix = suffix.clone();
            }
            Message::Assistant { content, usage } => {
                b.kind = BlockKind::Assistant;
                b.content = Some(content.clone());
                if let Some(u) = usage {
                    b.usage_total = Some(u.total_tokens);
                    b.usage_prompt = Some(u.prompt_tokens);
                    b.usage_completion = Some(u.completion_tokens);
                    b.usage_cached = Some(u.cached_tokens);
                    b.usage_cache_write = Some(u.cache_write_tokens);
                    b.usage_reasoning = Some(u.reasoning_tokens);
                }
            }
            Message::Reasoning { content } => {
                b.kind = BlockKind::Reasoning;
                b.content = Some(content.clone());
            }
            Message::ToolRequest {
                call_id,
                name,
                args,
                intent,
                text,
                first,
            } => {
                b.kind = BlockKind::ToolRequest;
                b.call_id = Some(call_id.clone());
                b.name = Some(name.clone());
                b.args = Some(args.clone());
                b.intent = Some(intent.clone());
                b.text = Some(text.clone());
                b.first = Some(*first);
            }
            Message::ToolResult {
                call_id,
                name,
                ok,
                result,
                memo,
                duration_ms,
            } => {
                b.kind = BlockKind::ToolResult;
                b.call_id = Some(call_id.clone());
                b.name = Some(name.clone());
                b.ok = Some(*ok);
                b.result = Some(result.clone());
                b.memo = memo.as_ref().map(|d| d.to_string());
                b.duration_ms = Some(*duration_ms as i64);
            }
        }
        b
    }

    /// Read one stored block back into a message. A row missing a field its kind needs is an error.
    pub fn from_block(b: &Block) -> Result<Self> {
        let need = |s: &Option<String>, what: &str| -> Result<String> {
            s.clone()
                .with_context(|| format!("a {} block carries no {what}", b.kind.as_str()))
        };
        Ok(match b.kind {
            BlockKind::User => Message::User {
                content: need(&b.content, "content")?,
                prefix: b.prefix.clone(),
                suffix: b.suffix.clone(),
            },
            BlockKind::Wake => Message::User {
                content: need(&b.content, "content")?,
                prefix: b.prefix.clone(),
                suffix: b.suffix.clone(),
            },
            BlockKind::Assistant => Message::Assistant {
                content: need(&b.content, "content")?,
                usage: b.usage_total.map(|total| UsageSummary {
                    total_tokens: total,
                    prompt_tokens: b.usage_prompt.unwrap_or(0),
                    cached_tokens: b.usage_cached.unwrap_or(0),
                    completion_tokens: b.usage_completion.unwrap_or(0),
                    reasoning_tokens: b.usage_reasoning.unwrap_or(0),
                    cache_write_tokens: b.usage_cache_write.unwrap_or(0),
                }),
            },
            BlockKind::Reasoning => Message::Reasoning {
                content: need(&b.content, "content")?,
            },
            BlockKind::ToolRequest => Message::ToolRequest {
                call_id: need(&b.call_id, "call_id")?,
                name: need(&b.name, "name")?,
                args: need(&b.args, "args")?,
                intent: b.intent.clone().unwrap_or_default(),
                text: b.text.clone().unwrap_or_default(),
                first: b.first.unwrap_or(false),
            },
            BlockKind::ToolResult => Message::ToolResult {
                call_id: need(&b.call_id, "call_id")?,
                name: need(&b.name, "name")?,
                ok: b.ok.unwrap_or(false),
                result: need(&b.result, "result")?,
                memo: b.memo.as_deref().and_then(|m| serde_json::from_str(m).ok()),
                duration_ms: b.duration_ms.unwrap_or(0).max(0) as u64,
            },
        })
    }
}

/// How loud a notice is. A notice is not a message: it is said to whoever is watching, never stored,
/// and never enters a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warn,
    Error,
}

// ---- the wire shapes ----

// One conversation message. The role is its identity, which is exactly serde's internally-tagged form.

// What a user message carries: text, or text plus images. `untagged`, so the text-only case stays
// byte-identical to before images existed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UserContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl UserContent {
    /// The text half. Image parts contribute nothing — this is what a
    /// text-only consumer (the context estimate, the transcript) should read.
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

    pub(crate) fn text_len(&self) -> usize {
        match self {
            Self::Text(t) => t.len(),
            Self::Parts(parts) => parts
                .iter()
                .map(|p| match p {
                    ContentPart::Text { text } => text.len(),
                    // An image costs roughly its payload; the URL length keeps the estimate monotone.
                    ContentPart::ImageUrl { image_url } => image_url.url.len(),
                })
                .sum(),
        }
    }

    pub fn has_images(&self) -> bool {
        matches!(self, Self::Parts(parts)
            if parts.iter().any(|p| matches!(p, ContentPart::ImageUrl { .. })))
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

/// One part of a multimodal user message: only the two the OpenAI-compatible wire defines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
    },
    /// A remote URL or a `data:` URI.
    ImageUrl {
        image_url: ImageUrl,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    /// OpenAI's resolution hint (`"auto"` / `"low"` / `"high"`). Some hosts
    /// reject `"original"` outright — that is `supports_image_detail_original`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum WireMessage {
    // System prompt. By convention the first array entry; plain text at heart.
    System {
        content: String,
    },
    // User input: plain text or an array of parts. Untagged, so the text-only wire shape is unchanged.
    User {
        content: UserContent,
    },
    // The model's reply. Either field may be empty: text only, tools only, or both.
    Assistant {
        // `None` in pure tool-call rounds must serialize as an explicit `null`, never an omitted field.
        #[serde(default, serialize_with = "serialize_content")]
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
        // Reasoning captured this round. Deliberately not part of the wire shape: `messages_to_wire`
        // decides per host whether and under which name it goes out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
    },
    // A tool result, paired by `tool_call_id`. Older gateways also want `name`; omitted until one
    // demands it.
    Tool {
        tool_call_id: String,
        content: String,
    },
}

impl WireMessage {
    /// A rough character count for the cheap context estimate; not a tokenizer.
    pub fn approx_chars(&self) -> usize {
        match self {
            WireMessage::User { content } => content.text_len(),
            WireMessage::Assistant {
                content,
                tool_calls,
                reasoning: _,
            } => {
                content.as_deref().unwrap_or("").len()
                    + tool_calls
                        .iter()
                        .map(|c| c.function.arguments.len() + c.function.name.len())
                        .sum::<usize>()
            }
            WireMessage::Tool { content, .. } => content.len(),
            WireMessage::System { content } => content.len(),
        }
    }
}

// `None` becomes JSON `null`, not an omitted field.
fn serialize_content<S: serde::Serializer>(v: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(t) => s.serialize_some(t),
        None => s.serialize_none(),
    }
}

// A tool call requested by the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    // Unique id of this call, generated by the model (e.g. `call_abc123`).
    pub id: String,
    // Always `"function"`: the field is required, and strict gateways reject its absence.
    #[serde(rename = "type", default = "function_kind")]
    pub kind: String,
    pub function: FunctionCall,
}

// Tool name plus arguments.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionCall {
    pub name: String,
    // **Raw JSON text**, not a parsed object: the wire sends a string, malformed arguments must not
    // break the parser, and a `Value` round-trip could reorder keys and cost the prefix cache.
    pub arguments: String,
}

// A tool definition: the manual we send. Request direction only, hence no `Deserialize`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionDef,
}

// The manual body. `parameters` is a JSON Schema.
#[derive(Debug, Clone, Serialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl FunctionCall {
    // Parse arguments. Failures carry readable context, not a bare serde error.
    pub fn arguments_json(&self) -> serde_json::Result<Value> {
        serde_json::from_str(&self.arguments)
    }
}

fn function_kind() -> String {
    "function".to_string()
}

impl ToolCall {
    // Build a call (`kind` filled here; callers never touch the constant).
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

    // Tool name (saves one `.function.` hop).
    pub fn name(&self) -> &str {
        &self.function.name
    }
    // Parse arguments. Failures carry readable context, not a bare serde error.
    pub fn arguments_json(&self) -> serde_json::Result<Value> {
        self.function.arguments_json()
    }
    /// The model's own "what am I about to do"; empty when it wrote none.
    pub fn intent(&self) -> String {
        self.arguments_json()
            .ok()
            .and_then(|v| v.get("intent").and_then(|i| i.as_str()).map(String::from))
            .unwrap_or_default()
    }
}

impl ToolDef {
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self {
            kind: "function",
            function: FunctionDef {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

// Everything sent to the model: the API is stateless, so this is its entire memory, resent every turn.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub messages: Vec<WireMessage>,
    pub tools: Vec<ToolDef>,
}

impl Context {
    pub fn new() -> Self {
        Self::default()
    }

    // Append a message (builder style, chainable).
    pub fn push(mut self, msg: WireMessage) -> Self {
        self.messages.push(msg);
        self
    }

    // Append a tool definition.
    pub fn tool(mut self, def: ToolDef) -> Self {
        self.tools.push(def);
        self
    }
}

// Token usage. The three totals are required; the `_details` extensions are optional.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    // Extension: reasoning tokens (thinking models only). Invisible but billed as output.
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
    // Extension: prefix cache hits. The lifeblood of agent-loop cost savings.
    #[serde(default)]
    pub cached_tokens: Option<u64>,
    // Prompt tokens written into the cache; a subset of `prompt_tokens`.
    #[serde(default)]
    pub cache_write_tokens: Option<u64>,
}

// Why the model stopped — maps to finish_reason; the loop's steering wheel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    // Finished speaking.
    #[default]
    Stop,
    // Cut off by max_tokens, mid-sentence.
    Length,
    // The loop's only continue condition: run the tools and ask again.
    ToolCalls,
    // User interrupt (Esc). The partial content received so far is usable.
    Interrupted,
    // The provider reported an error after the stream started.
    Error,
}

impl StopReason {
    /// Stable name for the `turns.stop_reason` column (snake_case, matching the rename).
    pub fn as_str(&self) -> &'static str {
        match self {
            StopReason::Stop => "stop",
            StopReason::Length => "length",
            StopReason::ToolCalls => "tool_calls",
            StopReason::Interrupted => "interrupted",
            StopReason::Error => "error",
        }
    }
}

// One complete model reply, assembled from deltas when streaming: the uniform type the loop consumes.
#[derive(Debug, Clone, Default)]
pub struct AssistantMessage {
    pub content: String,
    // Tool calls placed this round. Empty means the model just talked.
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: StopReason,
    pub usage: Usage,
    // Reasoning, when the model emits it. Not replayed in later turns.
    pub reasoning: Option<String>,
}

impl AssistantMessage {
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    // To `WireMessage::Assistant`. Empty content serializes as `None`, not `Some("")`.
    pub fn to_message(&self) -> WireMessage {
        WireMessage::Assistant {
            content: if self.content.is_empty() {
                None
            } else {
                Some(self.content.clone())
            },
            tool_calls: self.tool_calls.clone(),
            // Kept here so a later turn *can* replay it. Whether it actually
            // goes on the wire is `messages_to_wire`'s call, per host.
            reasoning: self.reasoning.clone().filter(|r| !r.is_empty()),
        }
    }
}
