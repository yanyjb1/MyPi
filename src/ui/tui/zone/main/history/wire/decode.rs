//! One block row, decoded; and the pairing of a tool exchange.

use serde_json::Value;

/// What a block is. The wire's `type` column, in this front end's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    User,
    Wake,
    Assistant,
    Reasoning,
    ToolRequest,
    ToolResult,
}

impl Kind {
    fn of(raw: &str) -> Option<Kind> {
        Some(match raw {
            "user" => Kind::User,
            "wake" => Kind::Wake,
            "assistant" => Kind::Assistant,
            "reasoning" => Kind::Reasoning,
            "tool_request" => Kind::ToolRequest,
            "tool_result" => Kind::ToolResult,
            _ => return None,
        })
    }
}

/// What one request cost. The four input columns mean what the request half wrote:
/// `prompt` is the whole input of that request, `cached` and `cache_write` are parts of it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt: u64,
    pub completion: u64,
    pub cached: u64,
    pub cache_write: u64,
}

impl Usage {
    /// The context this request carried: what went in plus what came back.
    pub fn context(self) -> u64 {
        self.prompt + self.completion
    }
}

/// One block row, every column this front end draws from.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub id: i64,
    pub kind: Kind,
    pub content: Option<String>,
    pub text: Option<String>,
    pub name: Option<String>,
    pub args: Option<String>,
    pub intent: Option<String>,
    pub call_id: Option<String>,
    pub first: bool,
    pub ok: Option<bool>,
    pub result: Option<String>,
    pub memo: Option<String>,
    pub duration_ms: i64,
    pub usage: Usage,
}

impl Block {
    /// Decode one row. An unknown `type` is a row this front end cannot draw.
    pub fn of(row: &Value) -> Option<Block> {
        let id = row.get("id")?.as_i64()?;
        let kind = Kind::of(row.get("type")?.as_str()?)?;
        Some(Block {
            id,
            kind,
            content: text_of(row, "content"),
            text: text_of(row, "text"),
            name: text_of(row, "name"),
            args: text_of(row, "args"),
            intent: text_of(row, "intent"),
            call_id: text_of(row, "call_id"),
            first: row.get("first").and_then(Value::as_bool).unwrap_or(false),
            ok: row.get("ok").and_then(Value::as_bool),
            result: text_of(row, "result"),
            memo: text_of(row, "memo"),
            duration_ms: row.get("duration_ms").and_then(Value::as_i64).unwrap_or(0),
            usage: Usage {
                prompt: count_of(row, "usage_prompt"),
                completion: count_of(row, "usage_completion"),
                cached: count_of(row, "usage_cached"),
                cache_write: count_of(row, "usage_cache_write"),
            },
        })
    }

    /// The tool result that answers this request, by call id.
    pub fn answers(&self, other: &Block) -> bool {
        self.kind == Kind::ToolRequest
            && other.kind == Kind::ToolResult
            && self.call_id.is_some()
            && self.call_id == other.call_id
    }
}

fn text_of(row: &Value, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::to_string)
}

fn count_of(row: &Value, key: &str) -> u64 {
    row.get(key).and_then(Value::as_u64).unwrap_or(0)
}
