//! One block row, decoded; and the pairing of a tool exchange.

use serde_json::Value;

/// What a block is. The wire's `kind` field, in this front end's words.
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

/// What one item of a call answered: its own outcome, and its failure text when it failed.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemOk {
    pub ok: bool,
    pub error: Option<String>,
}

impl ItemOk {
    /// One entry; a missing flag reads as failed, which is the safe direction.
    fn of(value: &Value) -> ItemOk {
        ItemOk {
            ok: value.get("ok").and_then(Value::as_bool).unwrap_or(false),
            error: value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    }
}

/// The outcomes a relayed result carries: one entry per item the call named.
pub fn items_of(value: &Value) -> Option<Vec<ItemOk>> {
    let list = value.as_array()?;
    Some(list.iter().map(ItemOk::of).collect())
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
    /// One entry per item a call named; `None` while it runs.
    pub ok: Option<Vec<ItemOk>>,
    pub result: Option<String>,
    pub memo: Option<String>,
    pub duration_ms: i64,
    pub truncated: Option<Truncation>,
}

/// How much a live result left behind when it was cut: the full text, not the part kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Truncation {
    pub lines: u64,
    pub bytes: u64,
}

impl Truncation {
    /// The relay's `{lines, bytes}` object, when this frame carries one.
    pub fn of(value: Option<&Value>) -> Option<Truncation> {
        let value = value?.as_object()?;
        Some(Truncation {
            lines: value.get("lines")?.as_u64()?,
            bytes: value.get("bytes")?.as_u64()?,
        })
    }
}

impl Block {
    /// Decode one row. An unknown `type` is a row this front end cannot draw.
    pub fn of(row: &Value) -> Option<Block> {
        let id = row.get("id")?.as_i64()?;
        let kind = Kind::of(row.get("kind")?.as_str()?)?;
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
            ok: row.get("ok").and_then(items_of),
            result: text_of(row, "result"),
            memo: text_of(row, "memo"),
            duration_ms: row.get("duration_ms").and_then(Value::as_i64).unwrap_or(0),
            truncated: None,
        })
    }
}

fn text_of(row: &Value, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::to_string)
}
