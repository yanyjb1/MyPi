//! OpenAI Responses: `input` items in, events out.

use crate::server::platform::stop::Stop;
use std::collections::BTreeMap;
use std::io::BufRead;

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::request::contract::Kind;
use crate::server::agent::request::live::Stream;
use crate::server::agent::request::message::{
    AssistantMessage, DEFAULT_EMPTY_REPLY, INTERRUPTED_REPLY, ToolDef, Usage, WireCall, WireMessage,
};
use crate::server::agent::request::protocol::blocks;
use crate::server::agent::request::protocol::stream::{self, Budget};
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

    fn budget(&self) -> Budget {
        Budget::default()
    }

    fn decode(
        &self,
        reader: Box<dyn BufRead>,
        out: &mut dyn Output,
        cancel: &Stop,
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
    cancel: &Stop,
) -> Result<AssistantMessage> {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut calls: BTreeMap<String, WireCall> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut usage = Usage::default();

    let end = stream::data(reader.as_mut(), cancel, |payload| {
        let event: Value = serde_json::from_str(payload)?;
        match event.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "response.output_text.delta" => {
                if let Some(d) = event.get("delta").and_then(|v| v.as_str()) {
                    content.push_str(d);
                    out.stream(Stream::text(d.to_string()));
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(d) = event.get("delta").and_then(|v| v.as_str()) {
                    reasoning.push_str(d);
                    out.stream(Stream::thinking(d.to_string()));
                }
            }
            "response.output_item.added" => {
                let Some(item) = event.get("item") else {
                    return Ok(true);
                };
                if item.get("type").and_then(|v| v.as_str()) != Some("function_call") {
                    return Ok(true);
                }
                let key = item
                    .get("id")
                    .or_else(|| item.get("call_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if key.is_empty() {
                    return Ok(true);
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
                    let (id, name) = (call.id.clone(), call.function.name.clone());
                    out.stream(Stream::arguments(id, name, d.to_string()));
                }
            }
            "response.output_item.done" => {
                let Some(item) = event.get("item") else {
                    return Ok(true);
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
        Ok(true)
    })?;
    let interrupted = end == stream::End::Canceled;

    let mut placed: Vec<WireCall> = order.into_iter().filter_map(|k| calls.remove(&k)).collect();
    placed.retain(|call| call.complete());
    if interrupted {
        placed.clear();
    }

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
