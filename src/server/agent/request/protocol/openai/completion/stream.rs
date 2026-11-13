//! Chat Completions: SSE deltas in, pieces and blocks out.

use crate::server::platform::stop::Stop;
use std::io::BufRead;

use anyhow::Context as _;
use serde::Deserialize;
use serde_json::Value;

use crate::server::agent::request::contract::Kind;
use crate::server::agent::request::live::Stream;
use crate::server::agent::request::message::{AssistantMessage, INTERRUPTED_REPLY, Usage};
use crate::server::agent::request::protocol::blocks;
use crate::server::agent::request::protocol::stitch::Calls;
use crate::server::agent::request::protocol::stream;
use crate::server::agent::request::protocol::{Output, broken_calls, fault, silence};

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
    out: &mut dyn Output,
    cancel: &Stop,
) -> anyhow::Result<AssistantMessage> {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut finish: Option<String> = None;
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
            let Some(piece) = delta.get(field).and_then(Value::as_str) else {
                continue;
            };
            if !piece.is_empty() {
                reasoning.push_str(piece);
                out.stream(Stream::thinking(piece.to_string()));
            }
            break;
        }

        if let Some(text) = delta.get("content").and_then(Value::as_str)
            && !text.is_empty()
        {
            content.push_str(text);
            out.stream(Stream::text(text.to_string()));
        }

        calls.feed(delta, &mut |call_id, name, text| {
            out.stream(Stream::arguments(
                call_id.to_string(),
                name.to_string(),
                text.to_string(),
            ));
        })?;
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            finish = Some(reason.to_string());
        }
        Ok(true)
    })?;
    let interrupted = end == stream::End::Canceled;

    let mut placed = calls.finish();
    let declared = placed.len();
    placed.retain(|call| call.complete());
    let dropped = declared - placed.len();
    // A reply the user cut short leaves no call behind: one call that never ran leaves a phantom.
    if interrupted {
        placed.clear();
    }
    if !reasoning.is_empty() {
        out.block(blocks::text(Kind::Reasoning, &reasoning));
    }
    // A call the model placed and the wire never closed is the upstream's fault, never a skip.
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
