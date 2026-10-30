//! Anthropic Messages: `messages` in, events out.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::request::contract::Kind;
use crate::server::agent::request::live::Stream;
use crate::server::agent::request::message::{
    AssistantMessage, ContentPart, DEFAULT_EMPTY_REPLY, INTERRUPTED_REPLY, ToolDef, Usage,
    UserContent, WireCall, WireMessage,
};
use crate::server::agent::request::protocol::anthropic::stream::read_event;
use crate::server::agent::request::protocol::blocks;
use crate::server::agent::request::protocol::{
    Endpoint, Output, Protocol, ProtocolRequest, WireRequest,
};

pub struct Messages;

impl Messages {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Messages {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for Messages {
    fn request(&self, endpoint: &Endpoint, req: &ProtocolRequest) -> Result<WireRequest> {
        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_output_tokens,
            "stream": req.stream,
            "messages": messages(&req.messages),
        });
        let system = system(&req.messages);
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if !req.tools.is_empty() {
            body["tools"] = json!(tools(&req.tools));
        }
        let mut headers = vec![("anthropic-version".to_string(), "2023-06-01".to_string())];
        if endpoint.auth_header {
            headers.push(("x-api-key".to_string(), endpoint.api_key.clone()));
        }
        Ok(WireRequest {
            url: format!("{}/messages", endpoint.base_url.trim_end_matches('/')),
            headers,
            body,
        })
    }

    fn decode(
        &self,
        reader: Box<dyn BufRead>,
        out: &mut dyn Output,
        cancel: &AtomicBool,
    ) -> Result<AssistantMessage> {
        decode(reader, out, cancel)
    }
}

fn system(messages: &[WireMessage]) -> String {
    messages
        .iter()
        .filter_map(|m| match m {
            WireMessage::System { content } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn messages(messages: &[WireMessage]) -> Vec<Value> {
    let mut out: Vec<(String, Vec<Value>)> = Vec::new();
    for m in messages {
        match m {
            WireMessage::System { .. } => {}
            WireMessage::User { content } => push(&mut out, "user", user_parts(content)),
            WireMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                let mut parts = Vec::new();
                if let Some(text) = content
                    && !text.is_empty()
                {
                    parts.push(json!({"type": "text", "text": text}));
                }
                for call in tool_calls {
                    let input = serde_json::from_str::<Value>(&call.function.arguments)
                        .unwrap_or_else(|_| json!({}));
                    parts.push(json!({
                        "type": "tool_use",
                        "id": &call.id,
                        "name": call.name(),
                        "input": input,
                    }));
                }
                if parts.is_empty() {
                    parts.push(json!({"type": "text", "text": ""}));
                }
                push(&mut out, "assistant", parts);
            }
            WireMessage::Tool {
                tool_call_id,
                content,
            } => push(
                &mut out,
                "user",
                vec![json!({
                    "type": "tool_result",
                    "tool_use_id": &tool_call_id,
                    "content": content,
                })],
            ),
        }
    }
    out.into_iter()
        .map(|(role, content)| json!({"role": role, "content": content}))
        .collect()
}

fn push(out: &mut Vec<(String, Vec<Value>)>, role: &str, parts: Vec<Value>) {
    match out.last_mut() {
        Some((last, content)) if last.as_str() == role => content.extend(parts),
        _ => out.push((role.to_string(), parts)),
    }
}

fn user_parts(content: &UserContent) -> Vec<Value> {
    match content {
        UserContent::Text(text) => vec![json!({"type": "text", "text": text})],
        UserContent::Parts(parts) => parts
            .iter()
            .map(|p| match p {
                ContentPart::Text { text } => json!({"type": "text", "text": text}),
                ContentPart::ImageUrl { image_url } => json!({
                    "type": "image",
                    "source": {"type": "url", "url": &image_url.url},
                }),
            })
            .collect(),
    }
}

fn tools(tools: &[ToolDef]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "name": &t.function.name,
                "description": &t.function.description,
                "input_schema": &t.function.parameters,
            })
        })
        .collect()
}

/// One content block as the stream builds it.
#[derive(Default)]
struct Chunk {
    role: Role,
    text: String,
    id: String,
    name: String,
    args: String,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Role {
    #[default]
    Text,
    Thinking,
    Tool,
}

fn decode(
    mut reader: Box<dyn BufRead>,
    out: &mut dyn Output,
    cancel: &AtomicBool,
) -> Result<AssistantMessage> {
    let mut chunks: BTreeMap<u64, Chunk> = BTreeMap::new();
    let mut order: Vec<u64> = Vec::new();
    let mut usage = Usage::default();
    let mut interrupted = false;

    while let Some(event) = read_event(reader.as_mut())? {
        if cancel.load(Ordering::Relaxed) {
            interrupted = true;
            break;
        }
        let Ok(data) = serde_json::from_str::<Value>(&event.data) else {
            continue;
        };
        let kind = event
            .event
            .as_deref()
            .or_else(|| data.get("type").and_then(|v| v.as_str()))
            .unwrap_or("");
        match kind {
            "message_start" => {
                if let Some(u) = data.get("message").and_then(|m| m.get("usage")) {
                    usage = usage_from(u);
                }
            }
            "content_block_start" => {
                let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                let Some(block) = data.get("content_block") else {
                    continue;
                };
                let mut chunk = Chunk::default();
                match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                    "text" => chunk.role = Role::Text,
                    "thinking" => chunk.role = Role::Thinking,
                    "tool_use" => {
                        chunk.role = Role::Tool;
                        chunk.id = block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        chunk.name = block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                    }
                    _ => continue,
                }
                if !chunks.contains_key(&index) {
                    order.push(index);
                }
                chunks.insert(index, chunk);
            }
            "content_block_delta" => {
                let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                let Some(chunk) = chunks.get_mut(&index) else {
                    continue;
                };
                let Some(delta) = data.get("delta") else {
                    continue;
                };
                let piece = delta
                    .get("text")
                    .or_else(|| delta.get("thinking"))
                    .or_else(|| delta.get("partial_json"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if piece.is_empty() {
                    continue;
                }
                chunk.text.push_str(piece);
                match chunk.role {
                    Role::Tool => {
                        chunk.args.push_str(piece);
                        let id = chunk.id.clone();
                        out.stream(Stream::Arguments {
                            call_id: id,
                            text: piece.to_string(),
                        });
                    }
                    Role::Thinking => out.stream(Stream::Thinking(piece.to_string())),
                    Role::Text => out.stream(Stream::Text(piece.to_string())),
                }
            }
            "message_delta" => {
                if let Some(output) = data
                    .get("usage")
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(Value::as_u64)
                {
                    usage.completion = output;
                }
            }
            "error" => anyhow::bail!("anthropic stream failed: {}", event.data),
            "message_stop" => break,
            _ => {}
        }
    }

    let mut content = String::new();
    let mut reasoning = String::new();
    let mut calls = Vec::new();
    for index in order {
        let Some(chunk) = chunks.remove(&index) else {
            continue;
        };
        match chunk.role {
            Role::Text => content.push_str(&chunk.text),
            Role::Thinking => reasoning.push_str(&chunk.text),
            Role::Tool => calls.push(WireCall::new(chunk.id, chunk.name, chunk.args)),
        }
    }

    if !reasoning.is_empty() {
        out.block(blocks::text(Kind::Reasoning, &reasoning));
    }
    if calls.is_empty() {
        let body = if !content.is_empty() {
            content.clone()
        } else if interrupted {
            INTERRUPTED_REPLY.to_string()
        } else {
            DEFAULT_EMPTY_REPLY.to_string()
        };
        out.block(blocks::assistant(&body));
    } else {
        if !content.is_empty() {
            out.block(blocks::assistant(&content));
        }
        for (i, call) in calls.iter().enumerate() {
            out.block(blocks::call(call, i == 0));
        }
    }

    Ok(AssistantMessage {
        content,
        calls,
        usage,
        reasoning: (!reasoning.is_empty()).then_some(reasoning),
        interrupted,
    })
}

fn usage_from(u: &Value) -> Usage {
    Usage {
        prompt: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
        completion: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
        cached: u
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_write: u
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        reasoning: 0,
    }
}
