//! DeepSeek: its own body and its own decode; the completion dialect is a different wire.

use crate::platform::stop::Stop;
use std::io::BufRead;

use anyhow::Result;
use serde_json::{Map, Value, json};

use crate::server::agent::request::contract::Kind;
use crate::server::agent::request::live::Stream;
use crate::server::agent::request::message::{
    AssistantMessage, ContentPart, INTERRUPTED_REPLY, ImagePart, Usage, UserContent, WireMessage,
};
use crate::server::agent::request::protocol::blocks;
use crate::server::agent::request::protocol::stitch::Calls;
use crate::server::agent::request::protocol::stream;
use crate::server::agent::request::protocol::{
    Effort, Endpoint, Output, Protocol, ProtocolRequest, WireRequest, broken_calls, fault, silence,
};

pub struct Deepseek;

impl Deepseek {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Deepseek {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for Deepseek {
    fn request(&self, endpoint: &Endpoint, req: &ProtocolRequest) -> Result<WireRequest> {
        let mut body = Map::new();
        body.insert("model".to_string(), json!(req.model));
        body.insert(
            "messages".to_string(),
            Value::Array(messages(&req.messages)),
        );
        body.insert("stream".to_string(), json!(req.stream));
        body.insert("max_tokens".to_string(), json!(req.max_output_tokens));
        // `Auto` names no rung: the field stays off the body and the host keeps its default.
        if req.effort != Effort::Auto {
            body.insert("reasoning_effort".to_string(), json!(req.effort.as_str()));
        }
        if !req.tools_json.as_array().is_some_and(Vec::is_empty) {
            body.insert("tools".to_string(), req.tools_json.as_ref().clone());
        }
        let mut headers = Vec::new();
        if endpoint.auth_header {
            headers.push((
                "Authorization".to_string(),
                format!("Bearer {}", endpoint.api_key),
            ));
        }
        Ok(WireRequest {
            url: format!(
                "{}/chat/completions",
                endpoint.base_url.trim_end_matches('/')
            ),
            headers,
            body: Value::Object(body),
        })
    }

    fn decode(
        &self,
        mut reader: Box<dyn BufRead>,
        out: &mut dyn Output,
        cancel: &Stop,
    ) -> Result<AssistantMessage> {
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut usage = Usage::default();
        let mut calls = Calls::default();
        let mut finish: Option<String> = None;

        let interrupted = stream::data(reader.as_mut(), cancel, |payload| {
            let chunk: Value = serde_json::from_str(payload)?;

            if let Some(u) = chunk.get("usage").filter(|v| !v.is_null()) {
                usage = read_usage(u);
            }

            let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) else {
                return Ok(true);
            };
            let delta = &choice["delta"];
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                finish = Some(reason.to_string());
            }

            if let Some(piece) = delta.get("reasoning_content").and_then(Value::as_str)
                && !piece.is_empty()
            {
                reasoning.push_str(piece);
                out.stream(Stream::thinking(piece.to_string()));
            }
            if let Some(piece) = delta.get("content").and_then(Value::as_str)
                && !piece.is_empty()
            {
                content.push_str(piece);
                out.stream(Stream::text(piece.to_string()));
            }
            calls.feed(delta, &mut |call_id, name, text| {
                out.stream(Stream::arguments(
                    call_id.to_string(),
                    name.to_string(),
                    text.to_string(),
                ));
            })?;
            Ok(true)
        })?;

        let mut placed = calls.finish();
        let declared = placed.len();
        placed.retain(|call| call.complete());
        let dropped = declared - placed.len();
        if interrupted {
            placed.clear();
        }
        if !reasoning.is_empty() {
            out.block(blocks::text(Kind::Reasoning, &reasoning));
        }
        if !interrupted && dropped > 0 {
            return Err(fault(out, broken_calls(dropped, finish.as_deref())));
        }
        if placed.is_empty() {
            let body = if !content.is_empty() {
                content.clone()
            } else if interrupted {
                INTERRUPTED_REPLY.to_string()
            } else {
                match silence(finish.as_deref()) {
                    Ok(text) => text.to_string(),
                    Err(why) => return Err(fault(out, why)),
                }
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
}

/// Every assistant turn carries `reasoning_content`; an empty string is the accepted placeholder.
fn messages(messages: &[WireMessage]) -> Vec<Value> {
    let mut out = Vec::new();
    for message in messages {
        match message {
            WireMessage::System { content } => out.push(json!({
                "role": "system",
                "content": content,
            })),
            WireMessage::User { content } => out.push(json!({
                "role": "user",
                "content": match content {
                    UserContent::Text(text) => json!(text),
                    UserContent::Parts(parts) => {
                        Value::Array(parts.iter().map(part).collect())
                    }
                },
            })),
            WireMessage::Assistant {
                content,
                tool_calls,
                reasoning,
            } => {
                let mut entry = Map::new();
                entry.insert("role".to_string(), json!("assistant"));
                entry.insert(
                    "content".to_string(),
                    match content {
                        Some(text) if !text.is_empty() => json!(text),
                        _ => Value::Null,
                    },
                );
                entry.insert(
                    "reasoning_content".to_string(),
                    json!(reasoning.as_deref().unwrap_or("")),
                );
                if !tool_calls.is_empty() {
                    entry.insert(
                        "tool_calls".to_string(),
                        json!(
                            tool_calls
                                .iter()
                                .map(|call| json!({
                                    "id": call.id,
                                    "type": "function",
                                    "function": {
                                        "name": call.function.name,
                                        "arguments": call.function.arguments,
                                    },
                                }))
                                .collect::<Vec<_>>()
                        ),
                    );
                }
                out.push(Value::Object(entry));
            }
            WireMessage::Tool {
                tool_call_id,
                content,
                images,
            } => {
                // A tool result with images becomes the parts array; text alone stays a string.
                let content = if images.is_empty() {
                    json!(content)
                } else {
                    let mut parts = vec![json!({ "type": "text", "text": content })];
                    parts.extend(images.iter().map(|image| image_part(image)));
                    Value::Array(parts)
                };
                out.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": content,
                }))
            }
        }
    }
    out
}
/// One content part as the DeepSeek wire takes it; `detail` rides along only when named.
fn part(part: &ContentPart) -> Value {
    match part {
        ContentPart::Text { text } => json!({ "type": "text", "text": text }),
        ContentPart::ImageUrl { image_url } => {
            let mut entry = json!({ "url": image_url.url });
            if let Some(detail) = &image_url.detail {
                entry["detail"] = json!(detail);
            }
            json!({ "type": "image_url", "image_url": entry })
        }
    }
}

/// One tool-result image as the DeepSeek wire takes it: a data URL the host decodes itself.
fn image_part(image: &ImagePart) -> Value {
    let url = format!("data:image/{};base64,{}", image.format, image.data);
    json!({ "type": "image_url", "image_url": { "url": url } })
}

fn read_usage(raw: &Value) -> Usage {
    let number = |key: &str| raw.get(key).and_then(Value::as_u64).unwrap_or(0);
    Usage {
        prompt: number("prompt_tokens"),
        completion: number("completion_tokens"),
        cached: number("prompt_cache_hit_tokens"),
        cache_write: 0,
        reasoning: raw
            .get("completion_tokens_details")
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}
