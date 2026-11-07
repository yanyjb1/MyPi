//! Chat Completions: SSE deltas in, pieces and blocks out.

use std::io::BufRead;
use std::sync::atomic::AtomicBool;

use anyhow::Context as _;
use serde::Deserialize;
use serde_json::Value;

use crate::server::agent::request::contract::Kind;
use crate::server::agent::request::live::Stream;
use crate::server::agent::request::message::{
    AssistantMessage, DEFAULT_EMPTY_REPLY, INTERRUPTED_REPLY, Usage, WireCall,
};
use crate::server::agent::request::protocol::Output;
use crate::server::agent::request::protocol::blocks;
use crate::server::agent::request::protocol::openai::completion::compat::Resolved;
use crate::server::agent::request::protocol::openai::completion::inband::{ScanEvent, scanner_for};
use crate::server::agent::request::protocol::openai::completion::markup::TemplateMarkupFilter;
use crate::server::agent::request::protocol::stitch::Calls;
use crate::server::agent::request::protocol::stream;

#[derive(Debug, Deserialize)]
pub(crate) struct ApiUsage {
    #[serde(default)]
    pub(crate) prompt_tokens: u64,
    #[serde(default)]
    pub(crate) completion_tokens: u64,
    #[serde(default)]
    pub(crate) completion_tokens_details: Option<CompletionDetails>,
    #[serde(default)]
    pub(crate) prompt_tokens_details: Option<PromptDetails>,
    #[serde(default)]
    pub(crate) cached_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) prompt_cache_hit_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) cache_creation_input_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompletionDetails {
    #[serde(default)]
    pub(crate) reasoning_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PromptDetails {
    #[serde(default)]
    pub(crate) cached_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) cache_write_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) cache_creation_input_tokens: Option<u64>,
}

impl From<ApiUsage> for Usage {
    fn from(u: ApiUsage) -> Self {
        let cached = [
            u.cached_tokens,
            u.prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens),
            u.prompt_cache_hit_tokens,
            u.cache_read_input_tokens,
        ]
        .into_iter()
        .flatten()
        .find(|n| *n > 0)
        .unwrap_or(0);
        let written = [
            u.prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cache_write_tokens),
            u.prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cache_creation_input_tokens),
            u.cache_creation_input_tokens,
        ]
        .into_iter()
        .flatten()
        .find(|n| *n > 0)
        .unwrap_or(0);
        Usage {
            prompt: u.prompt_tokens,
            completion: u.completion_tokens,
            cached,
            cache_write: written,
            reasoning: u
                .completion_tokens_details
                .and_then(|d| d.reasoning_tokens)
                .unwrap_or(0),
        }
    }
}

pub(crate) fn decode(
    mut reader: Box<dyn BufRead>,
    compat: &Resolved,
    out: &mut dyn Output,
    cancel: &AtomicBool,
) -> anyhow::Result<AssistantMessage> {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut finish: Option<String> = None;
    let mut scanner = scanner_for(compat);
    let mut healed: Vec<WireCall> = Vec::new();
    let mut content_filter = TemplateMarkupFilter::for_compat(compat);
    let mut reasoning_filter = TemplateMarkupFilter::for_compat(compat);
    let mut usage = Usage::default();
    let mut calls = Calls::default();

    let end = stream::data(reader.as_mut(), cancel, |payload| {
        let chunk: Value = serde_json::from_str(payload)
            .with_context(|| format!("malformed stream chunk: {payload}"))?;

        if let Some(u) = chunk.get("usage").filter(|v| !v.is_null())
            && let Ok(parsed) = serde_json::from_value::<ApiUsage>(u.clone())
        {
            usage = Usage::from(parsed);
        }

        let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) else {
            return Ok(true);
        };
        let delta = &choice["delta"];

        for field in ["reasoning_content", "reasoning", "reasoning_text"] {
            let Some(raw) = delta.get(field).and_then(Value::as_str) else {
                continue;
            };
            let fresh = if compat.reasoning_deltas_may_be_cumulative && !reasoning.is_empty() {
                if raw.len() > reasoning.len() && raw.starts_with(reasoning.as_str()) {
                    raw[reasoning.len()..].to_string()
                } else if raw == reasoning.as_str() {
                    String::new()
                } else {
                    raw.to_string()
                }
            } else {
                raw.to_string()
            };
            let mut emitted = String::new();
            reasoning_filter.feed(&fresh, &mut emitted);
            if !emitted.is_empty() {
                reasoning.push_str(&emitted);
                out.stream(Stream::thinking(emitted));
            }
            break;
        }

        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            let mut emitted = String::new();
            content_filter.feed(text, &mut emitted);
            if !emitted.is_empty() {
                let structured = delta
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty());
                match scanner.as_mut() {
                    Some(scanner) => {
                        for event in scanner.feed(&emitted) {
                            match event {
                                ScanEvent::Text(t) => {
                                    content.push_str(&t);
                                    out.stream(Stream::text(t));
                                }
                                ScanEvent::Thinking(t) => {
                                    reasoning.push_str(&t);
                                    out.stream(Stream::thinking(t));
                                }
                                ScanEvent::ToolCall { name, arguments } => {
                                    if !structured {
                                        healed.push(WireCall::new(
                                            format!("call_healed_{}", healed.len()),
                                            name,
                                            arguments,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    None => {
                        content.push_str(&emitted);
                        out.stream(Stream::text(emitted));
                    }
                }
            }
        }

        calls.feed(delta, &mut |call_id, text| {
            out.stream(Stream::arguments(call_id.to_string(), text.to_string()));
        });
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            finish = Some(reason.to_string());
        }
        Ok(true)
    })?;
    let interrupted = end == stream::End::Canceled;

    if !interrupted && let Some(scanner) = scanner.as_mut() {
        for event in scanner.flush() {
            match event {
                ScanEvent::Text(t) => {
                    content.push_str(&t);
                    out.stream(Stream::text(t));
                }
                ScanEvent::Thinking(t) => {
                    reasoning.push_str(&t);
                    out.stream(Stream::thinking(t));
                }
                ScanEvent::ToolCall { name, arguments } => healed.push(WireCall::new(
                    format!("call_healed_{}", healed.len()),
                    name,
                    arguments,
                )),
            }
        }
    }

    if !interrupted {
        let mut tail = String::new();
        content_filter.flush(&mut tail);
        if !tail.is_empty() {
            content.push_str(&tail);
            out.stream(Stream::text(tail));
        }
        let mut thinking = String::new();
        reasoning_filter.flush(&mut thinking);
        if !thinking.is_empty() {
            reasoning.push_str(&thinking);
            out.stream(Stream::thinking(thinking));
        }
    }

    if !interrupted
        && compat.empty_length_finish_is_context_error
        && content.is_empty()
        && reasoning.is_empty()
        && finish.as_deref() == Some("length")
    {
        anyhow::bail!("the model stopped on `length` with no output at all");
    }

    let mut placed = calls.finish();
    placed.extend(healed);

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
