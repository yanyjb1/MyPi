use std::collections::BTreeMap;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::request::content::BlockSink;
use crate::server::agent::request::message::{
    AssistantMessage, ContentPart, DEFAULT_EMPTY_REPLY, INTERRUPTED_REPLY, StopReason, ToolCall,
    ToolDef, Usage, UserContent, WireMessage,
};
use crate::server::agent::request::protocols::anthropic::stream::read_event;
use crate::server::agent::request::protocols::openai::stream::{
    assistant_block, text_block, tool_block,
};
use crate::server::agent::request::protocols::{Protocol, ProtocolRequest, WireRequest};
use crate::server::agent::request::transport::client::Client;
use crate::server::storage::BlockKind;

pub struct Messages {
    _model: String,
}

impl Messages {
    pub fn new(model: &str) -> Self {
        Self {
            _model: model.to_string(),
        }
    }
}

impl Protocol for Messages {
    fn request(&self, client: &Client, req: &ProtocolRequest) -> Result<WireRequest> {
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
        Ok(WireRequest {
            url: format!("{}/messages", client.base_url.trim_end_matches('/')),
            headers: vec![
                ("x-api-key".to_string(), client.api_key.clone()),
                ("anthropic-version".to_string(), "2023-06-01".to_string()),
            ],
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
                    "content": &content,
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

#[derive(Default)]
struct Block {
    kind: u8,
    text: String,
    id: String,
    name: String,
    args: String,
}

fn decode(
    mut reader: Box<dyn BufRead>,
    sink: &mut dyn BlockSink,
    cancel: &AtomicBool,
) -> Result<AssistantMessage> {
    let mut blocks: BTreeMap<u64, Block> = BTreeMap::new();
    let mut order: Vec<u64> = Vec::new();
    let mut usage = Usage::default();
    let mut stop_reason: Option<String> = None;
    let mut interrupted = false;

    while let Some(event) = read_event(&mut reader)? {
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
                let index = data.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                let Some(block) = data.get("content_block") else {
                    continue;
                };
                let mut b = Block::default();
                match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                    "text" => b.kind = 0,
                    "thinking" => b.kind = 1,
                    "tool_use" => {
                        b.kind = 2;
                        b.id = block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        b.name = block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                    }
                    _ => continue,
                }
                if !blocks.contains_key(&index) {
                    order.push(index);
                }
                blocks.insert(index, b);
            }
            "content_block_delta" => {
                let index = data.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                let Some(block) = blocks.get_mut(&index) else {
                    continue;
                };
                let Some(delta) = data.get("delta") else {
                    continue;
                };
                let d = delta
                    .get("text")
                    .or_else(|| delta.get("thinking"))
                    .or_else(|| delta.get("partial_json"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if d.is_empty() {
                    continue;
                }
                block.text.push_str(d);
                if block.kind == 2 {
                    block.args.push_str(d);
                } else {
                    let kind = if block.kind == 1 {
                        BlockKind::Reasoning
                    } else {
                        BlockKind::Assistant
                    };
                    if !sink.block(text_block(kind, d), false) {
                        interrupted = true;
                        break;
                    }
                }
            }
            "message_delta" => {
                if let Some(reason) = data.get("delta").and_then(|d| d.get("stop_reason")) {
                    stop_reason = reason.as_str().map(str::to_string);
                }
                if let Some(u) = data.get("usage") {
                    let output = u.get("output_tokens").and_then(|v| v.as_u64());
                    if let Some(output) = output {
                        usage.completion_tokens = output;
                    }
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
        let Some(block) = blocks.remove(&index) else {
            continue;
        };
        match block.kind {
            0 => {
                content.push_str(&block.text);
            }
            1 => {
                reasoning.push_str(&block.text);
            }
            _ => {
                calls.push(ToolCall::new(block.id, block.name, block.args));
            }
        }
    }

    let stop = if interrupted {
        StopReason::Interrupted
    } else if !calls.is_empty() {
        StopReason::ToolCalls
    } else {
        match stop_reason.as_deref() {
            Some("max_tokens") => StopReason::Length,
            Some("tool_use") => StopReason::ToolCalls,
            _ => StopReason::Stop,
        }
    };

    if calls.is_empty() && content.is_empty() {
        let text = if interrupted {
            INTERRUPTED_REPLY
        } else {
            DEFAULT_EMPTY_REPLY
        };
        let _ = sink.block(assistant_block(text, &usage), true);
    } else if calls.is_empty() {
        let _ = sink.block(assistant_block(&content, &usage), true);
    } else {
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
        total_tokens: u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0)
            + u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        reasoning_tokens: None,
        cached_tokens: u.get("cache_read_input_tokens").and_then(|v| v.as_u64()),
        cache_write_tokens: u
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64()),
    }
}
