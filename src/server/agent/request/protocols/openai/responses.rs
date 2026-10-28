use std::collections::BTreeMap;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::request::content::BlockSink;
use crate::server::agent::request::message::{
    AssistantMessage, DEFAULT_EMPTY_REPLY, INTERRUPTED_REPLY, StopReason, ToolCall, Usage,
    WireMessage,
};
use crate::server::agent::request::protocols::openai::stream::{
    assistant_block, text_block, tool_block,
};
use crate::server::agent::request::protocols::{Protocol, ProtocolRequest, WireRequest};
use crate::server::agent::request::transport::client::Client;
use crate::server::storage::BlockKind;

pub struct Responses;

impl Responses {
    pub fn new(_model: &str) -> Self {
        Self
    }
}

impl Protocol for Responses {
    fn request(&self, client: &Client, req: &ProtocolRequest) -> Result<WireRequest> {
        let mut body = json!({
            "model": req.model,
            "input": input(&req.messages),
            "max_output_tokens": req.max_output_tokens,
            "stream": req.stream,
        });
        if !req.tools.is_empty() {
            body["tools"] = json!(tools(&req.tools));
        }
        Ok(WireRequest {
            url: format!("{}/responses", client.base_url.trim_end_matches('/')),
            headers: vec![(
                "Authorization".to_string(),
                format!("Bearer {}", client.api_key),
            )],
            body,
        })
    }

    fn decode(
        &self,
        reader: Box<dyn BufRead>,
        sink: &mut dyn BlockSink,
        cancel: &AtomicBool,
    ) -> Result<AssistantMessage> {
        decode(reader, sink, cancel)
    }

    fn compat_json(&self) -> Option<String> {
        None
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

fn tools(tools: &[crate::server::agent::request::message::ToolDef]) -> Vec<Value> {
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
    sink: &mut dyn BlockSink,
    cancel: &AtomicBool,
) -> Result<AssistantMessage> {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut calls: BTreeMap<String, ToolCall> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut usage = Usage::default();
    let mut interrupted = false;
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                if cancel.load(Ordering::Relaxed) {
                    interrupted = true;
                }
                break;
            }
            Ok(_) => {}
            Err(e) => {
                if cancel.load(Ordering::Relaxed) {
                    interrupted = true;
                    break;
                }
                return Err(e.into());
            }
        }
        let line = line.trim_end_matches(['\r', '\n']);
        let Some(payload) = line.strip_prefix("data: ") else {
            continue;
        };
        if payload == "[DONE]" {
            break;
        }
        let event: Value = serde_json::from_str(payload)?;
        match event.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "response.output_text.delta" => {
                if let Some(d) = event.get("delta").and_then(|v| v.as_str()) {
                    content.push_str(d);
                    if !sink.block(text_block(BlockKind::Assistant, d), false) {
                        interrupted = true;
                        break;
                    }
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(d) = event.get("delta").and_then(|v| v.as_str()) {
                    reasoning.push_str(d);
                    if !sink.block(text_block(BlockKind::Reasoning, d), false) {
                        interrupted = true;
                        break;
                    }
                }
            }
            "response.output_item.added" => {
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
                    calls.insert(key, ToolCall::new(id, name, args));
                }
            }
            "response.function_call_arguments.delta" => {
                let key = event
                    .get("item_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if let Some(call) = calls.get_mut(&key)
                    && let Some(d) = event.get("delta").and_then(|v| v.as_str())
                {
                    call.function.arguments.push_str(d);
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

    let calls: Vec<ToolCall> = order.into_iter().filter_map(|k| calls.remove(&k)).collect();
    let stop = if interrupted {
        StopReason::Interrupted
    } else if calls.is_empty() {
        StopReason::Stop
    } else {
        StopReason::ToolCalls
    };

    if !reasoning.is_empty() {
        let _ = sink.block(text_block(BlockKind::Reasoning, &reasoning), true);
    }
    if calls.is_empty() {
        let text = if !content.is_empty() {
            content.clone()
        } else if interrupted {
            INTERRUPTED_REPLY.to_string()
        } else {
            DEFAULT_EMPTY_REPLY.to_string()
        };
        let _ = sink.block(assistant_block(&text, &usage), true);
    } else {
        if !content.is_empty() {
            let _ = sink.block(assistant_block(&content, &usage), true);
        }
        for (i, call) in calls.iter().enumerate() {
            let _ = sink.block(tool_block(call, i == 0), true);
        }
    }

    Ok(AssistantMessage {
        content,
        tool_calls: calls,
        stop_reason: stop,
        usage,
        reasoning: (!reasoning.is_empty()).then_some(reasoning),
    })
}

fn usage_from(u: &Value) -> Usage {
    Usage {
        prompt_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        completion_tokens: u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        total_tokens: u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        reasoning_tokens: u
            .get("output_tokens_details")
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(|v| v.as_u64()),
        cached_tokens: u
            .get("input_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|v| v.as_u64()),
        cache_write_tokens: None,
    }
}
