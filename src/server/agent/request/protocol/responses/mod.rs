//! OpenAI Responses: `input` items in, events out.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::request::contract::Kind;
use crate::server::agent::request::live::Stream;
use crate::server::agent::request::message::{
    AssistantMessage, DEFAULT_EMPTY_REPLY, INTERRUPTED_REPLY, ToolDef, Usage, WireCall, WireMessage,
};
use crate::server::agent::request::protocol::blocks;
use crate::server::agent::request::protocol::sse::{self, Frame};
use crate::server::agent::request::protocol::{
    Endpoint, Output, Protocol, ProtocolRequest, WireRequest,
};

pub struct Responses;

impl Responses {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Responses {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for Responses {
    fn request(&self, endpoint: &Endpoint, req: &ProtocolRequest) -> Result<WireRequest> {
        let mut body = json!({
            "model": req.model,
            "input": input(&req.messages),
            "max_output_tokens": req.max_output_tokens,
            "stream": req.stream,
        });
        if !req.tools.is_empty() {
            body["tools"] = json!(tools(&req.tools));
        }
        let mut headers = Vec::new();
        if endpoint.auth_header {
            headers.push((
                "Authorization".to_string(),
                format!("Bearer {}", endpoint.api_key),
            ));
        }
        Ok(WireRequest {
            url: format!("{}/responses", endpoint.base_url.trim_end_matches('/')),
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

fn input(messages: &[WireMessage]) -> Vec<Value> {
    let mut out = Vec::new();
    for m in messages {
        match m {
            WireMessage::System { content } => {
                out.push(json!({"role": "system", "content": content}));
            }
            WireMessage::User { content } => {
                out.push(json!({"role": "user", "content": content}));
            }
            WireMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                if let Some(text) = content {
                    out.push(json!({"role": "assistant", "content": text}));
                }
                for call in tool_calls {
                    out.push(json!({
                        "type": "function_call",
                        "call_id": &call.id,
                        "name": call.name(),
                        "arguments": &call.function.arguments,
                    }));
                }
            }
            WireMessage::Tool {
                tool_call_id,
                content,
            } => {
                out.push(json!({
                    "type": "function_call_output",
                    "call_id": &tool_call_id,
                    "output": &content,
                }));
            }
        }
    }
    out
}

fn tools(tools: &[ToolDef]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "name": &t.function.name,
                "description": &t.function.description,
                "parameters": &t.function.parameters,
            })
        })
        .collect()
}

fn decode(
    mut reader: Box<dyn BufRead>,
    out: &mut dyn Output,
    cancel: &AtomicBool,
) -> Result<AssistantMessage> {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut calls: BTreeMap<String, WireCall> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut usage = Usage::default();
    let mut interrupted = false;
    let mut buf: Vec<u8> = Vec::new();

    loop {
        match sse::read(reader.as_mut(), &mut buf) {
            Ok(0) => {
                interrupted = cancel.load(Ordering::Relaxed);
                break;
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                if cancel.load(Ordering::Relaxed) {
                    interrupted = true;
                    break;
                }
                continue;
            }
            Err(e) => {
                if cancel.load(Ordering::Relaxed) {
                    interrupted = true;
                    break;
                }
                return Err(e.into());
            }
        }

        let payload = match sse::frame(&String::from_utf8_lossy(&buf))? {
            Frame::Skip => continue,
            Frame::Done => break,
            Frame::Payload(p) => p,
        };
        let event: Value = serde_json::from_str(&payload)?;
        match event.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "response.output_text.delta" => {
                if let Some(d) = event.get("delta").and_then(|v| v.as_str()) {
                    content.push_str(d);
                    out.stream(Stream::Text(d.to_string()));
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(d) = event.get("delta").and_then(|v| v.as_str()) {
                    reasoning.push_str(d);
                    out.stream(Stream::Thinking(d.to_string()));
                }
            }
            "response.output_item.added" => {
                let Some(item) = event.get("item") else {
                    continue;
                };
                if item.get("type").and_then(|v| v.as_str()) != Some("function_call") {
                    continue;
                }
                let key = item
                    .get("id")
                    .or_else(|| item.get("call_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if key.is_empty() {
                    continue;
                }
                let id = item
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&key)
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let args = item
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !calls.contains_key(&key) {
                    order.push(key.clone());
                }
                calls.insert(key, WireCall::new(id, name, args));
            }
            "response.function_call_arguments.delta" => {
                let key = event
                    .get("item_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if let Some(d) = event.get("delta").and_then(|v| v.as_str())
                    && let Some(call) = calls.get_mut(&key)
                {
                    call.function.arguments.push_str(d);
                    let id = call.id.clone();
                    out.stream(Stream::Arguments {
                        call_id: id,
                        text: d.to_string(),
                    });
                }
            }
            "response.output_item.done" => {
                let Some(item) = event.get("item") else {
                    continue;
                };
                if item.get("type").and_then(|v| v.as_str()) == Some("function_call") {
                    let key = item
                        .get("id")
                        .or_else(|| item.get("call_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if let Some(call) = calls.get_mut(&key)
                        && let Some(args) = item.get("arguments").and_then(|v| v.as_str())
                    {
                        call.function.arguments = args.to_string();
                    }
                }
            }
            "response.completed" => {
                if let Some(u) = event.get("response").and_then(|r| r.get("usage")) {
                    usage = usage_from(u);
                }
            }
            "response.failed" | "error" => {
                anyhow::bail!("responses stream failed: {payload}");
            }
            _ => {}
        }
    }

    let placed: Vec<WireCall> = order.into_iter().filter_map(|k| calls.remove(&k)).collect();

    if !reasoning.is_empty() {
        out.block(blocks::text(Kind::Reasoning, &reasoning));
    }
    if placed.is_empty() {
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
        for (i, call) in placed.iter().enumerate() {
            out.block(blocks::call(call, i == 0));
        }
    }

    Ok(AssistantMessage {
        content,
        calls: placed,
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
            .get("input_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_write: 0,
        reasoning: u
            .get("output_tokens_details")
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}
