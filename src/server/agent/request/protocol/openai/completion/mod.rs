//! Chat Completions: the standard body, auth, and decode. No vendor switches.

use crate::platform::stop::Stop;
use std::io::BufRead;

use anyhow::Result;

use crate::server::agent::request::message::AssistantMessage;
use crate::server::agent::request::protocol::{
    self, Endpoint, Output, Protocol, ProtocolRequest, WireRequest,
};
use crate::server::config::Effort;

pub mod stream;

pub struct Completion;

impl Completion {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Completion {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for Completion {
    fn request(&self, endpoint: &Endpoint, req: &ProtocolRequest) -> Result<WireRequest> {
        // A stored assistant turn's thinking is not serialized: one history is one size.
        let messages =
            serde_json::to_value(&req.messages).expect("Message serialization is infallible");
        let messages = with_tool_images(messages);
        let body = body_json(
            &req.model,
            req.max_output_tokens,
            req.stream,
            messages,
            req.tools_json.as_ref(),
            req.effort,
        );
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
            body,
        })
    }

    fn decode(
        &self,
        reader: Box<dyn BufRead>,
        out: &mut dyn Output,
        cancel: &Stop,
    ) -> Result<AssistantMessage> {
        protocol::openai::completion::stream::decode(reader, out, cancel)
    }
}

/// The one construction point for a request body: `model`, the messages, streaming with its usage,
/// the token ceiling, and the tools. Nothing a vendor invented.
fn body_json(
    model: &str,
    max_tokens: u32,
    stream: bool,
    messages: serde_json::Value,
    tools: &serde_json::Value,
    effort: Effort,
) -> serde_json::Value {
    use serde_json::{Map, Value, json};

    let mut body: Map<String, Value> = Map::new();
    body.insert("model".to_string(), Value::String(model.to_string()));
    body.insert("messages".to_string(), messages);
    body.insert("stream".to_string(), Value::Bool(stream));
    // Without this the stream names no usage at all, and there is no bill to keep.
    if stream {
        body.insert(
            "stream_options".to_string(),
            json!({ "include_usage": true }),
        );
    }
    body.insert("max_completion_tokens".to_string(), json!(max_tokens));
    // `Auto` names no rung: the field stays off the body and the host keeps its default.
    if effort != Effort::Auto {
        body.insert("reasoning_effort".to_string(), json!(effort.as_str()));
    }
    // An empty roster omits the field: strict hosts reject `"tools": []`.
    if tools.as_array().is_some_and(|list| !list.is_empty()) {
        body.insert("tools".to_string(), tools.clone());
    }
    Value::Object(body)
}

/// A tool result carrying images becomes the standard content-parts array; text alone stays a
/// string, so a text-only request keeps its bytes untouched.
fn with_tool_images(mut messages: serde_json::Value) -> serde_json::Value {
    use serde_json::{Value, json};

    let Some(list) = messages.as_array_mut() else {
        return messages;
    };
    for entry in list.iter_mut() {
        let Some(object) = entry.as_object_mut() else {
            continue;
        };
        if object.get("role").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        let Some(raw) = object.remove("images") else {
            continue;
        };
        let images = raw.as_array().cloned().unwrap_or_default();
        if images.is_empty() {
            continue;
        }
        let text = object
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut parts = vec![json!({ "type": "text", "text": text })];
        for image in images {
            let format = image.get("format").and_then(Value::as_str).unwrap_or("png");
            let data = image
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default();
            parts.push(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:image/{format};base64,{data}") },
            }));
        }
        object.insert("content".to_string(), Value::Array(parts));
    }
    messages
}
