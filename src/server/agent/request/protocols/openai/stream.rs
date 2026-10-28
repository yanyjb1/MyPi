use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use serde::Deserialize;
use serde_json::Value;

use crate::server::agent::request::content::BlockSink;
use crate::server::agent::request::message::{
    AssistantMessage, DEFAULT_EMPTY_REPLY, INTERRUPTED_REPLY, StopReason, ToolCall, Usage,
};
use crate::server::agent::request::protocols::openai::completion::compat::Resolved;
use crate::server::agent::request::protocols::openai::inband::{ScanEvent, scanner_for};
use crate::server::agent::request::protocols::openai::markup::TemplateMarkupFilter;
use crate::server::agent::request::retry::Failure;
use crate::server::storage::{BlockKind, NewBlock};

#[derive(Debug, Deserialize)]
pub(crate) struct ApiUsage {
    pub(crate) prompt_tokens: u64,
    pub(crate) completion_tokens: u64,
    pub(crate) total_tokens: u64,
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
        .find(|n| *n > 0);
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
        .find(|n| *n > 0);
        Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
            reasoning_tokens: u.completion_tokens_details.and_then(|d| d.reasoning_tokens),
            cached_tokens: cached,
            cache_write_tokens: written,
        }
    }
}

#[derive(Default)]
pub(crate) struct ToolCallAccum {
    pending: Vec<(u64, ToolCall)>,
}

impl ToolCallAccum {
    pub(crate) fn feed(&mut self, delta: &Value) {
        let Some(calls) = delta.get("tool_calls").and_then(|v| v.as_array()) else {
            return;
        };
        for raw in calls {
            let index = raw.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            let entry = match self.pending.iter_mut().find(|(i, _)| *i == index) {
                Some((_, call)) => call,
                None => {
                    self.pending.push((
                        index,
                        ToolCall::new(String::new(), String::new(), String::new()),
                    ));
                    &mut self.pending.last_mut().expect("just pushed").1
                }
            };
            if let Some(id) = raw.get("id").and_then(|v| v.as_str())
                && !id.is_empty()
            {
                entry.id = id.to_string();
            }
            if let Some(f) = raw.get("function") {
                if let Some(n) = f.get("name").and_then(|v| v.as_str())
                    && !n.is_empty()
                {
                    entry.function.name = n.to_string();
                }
                if let Some(a) = f.get("arguments").and_then(|v| v.as_str()) {
                    entry.function.arguments.push_str(a);
                }
            }
        }
    }

    pub(crate) fn finish(self) -> Vec<ToolCall> {
        let mut v = self.pending;
        v.sort_by_key(|(i, _)| *i);
        v.into_iter().map(|(_, c)| c).collect()
    }
}

pub(crate) fn parse_finish_reason(reason: Option<String>) -> StopReason {
    match reason.as_deref() {
        Some("length") => StopReason::Length,
        Some("tool_calls") | Some("function_call") => StopReason::ToolCalls,
        Some("error") => StopReason::Error,
        _ => StopReason::Stop,
    }
}

pub(crate) fn decode_completion(
    mut reader: Box<dyn BufRead>,
    compat: &Resolved,
    sink: &mut dyn BlockSink,
    cancel: &AtomicBool,
) -> anyhow::Result<AssistantMessage> {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut finish_reason: Option<String> = None;
    let mut scanner = scanner_for(compat);
    let mut healed_calls: Vec<ToolCall> = Vec::new();
    let mut content_filter = TemplateMarkupFilter::for_compat(compat);
    let mut reasoning_filter = TemplateMarkupFilter::for_compat(compat);
    let mut usage = Usage::default();
    let mut interrupted = false;
    let mut tool_calls = ToolCallAccum::default();

    let mut pending: Vec<u8> = Vec::new();
    let started = Instant::now();
    let first_budget = compat
        .stream_first_event_timeout_ms
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis);
    let mut got_first = false;
    let idle_budget = compat
        .stream_idle_timeout_ms
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis);
    let mut last_event = Instant::now();

    loop {
        pending.clear();
        match reader.read_until(b'\n', &mut pending) {
            Ok(0) => {
                if cancel.load(Ordering::Relaxed) {
                    interrupted = true;
                }
                break;
            }
            Ok(_) => {}
            Err(e) if is_timeout(&e) => {
                if cancel.load(Ordering::Relaxed) {
                    interrupted = true;
                    break;
                }
                if !got_first
                    && let Some(budget) = first_budget
                    && started.elapsed() > budget
                {
                    anyhow::bail!("no first event from upstream within {budget:?}");
                }
                if got_first
                    && let Some(budget) = idle_budget
                    && last_event.elapsed() > budget
                {
                    anyhow::bail!("no new event from upstream within {budget:?}");
                }
                continue;
            }
            Err(e) => {
                if cancel.load(Ordering::Relaxed) {
                    interrupted = true;
                    break;
                }
                return Err(e).context("stream read interrupted");
            }
        }

        let line = String::from_utf8_lossy(&pending);
        let line = line.strip_suffix('\n').unwrap_or(&line);
        let line = line.strip_suffix('\r').unwrap_or(line);

        if let Some(err) = line.strip_prefix("__WORKER_ERROR__") {
            if let Some(rest) = err.strip_prefix("__HTTP__") {
                let (status, body) = match rest.split_once('\n') {
                    Some((s, b)) => (s, b),
                    None => (rest, ""),
                };
                if let Ok(status) = status.parse::<u16>() {
                    return Err(anyhow::Error::new(Failure::Http {
                        status,
                        body: body.to_string(),
                    }));
                }
            }
            return Err(anyhow::Error::new(Failure::NoResponse {
                source: err.to_string(),
            }));
        }

        let Some(payload) = line.strip_prefix("data: ") else {
            continue;
        };
        if payload == "[DONE]" {
            break;
        }
        got_first = true;
        last_event = Instant::now();

        let chunk: Value = serde_json::from_str(payload)
            .with_context(|| format!("malformed stream chunk: {payload}"))?;

        if let Some(u) = chunk.get("usage").filter(|v| !v.is_null())
            && let Ok(parsed) = serde_json::from_value::<ApiUsage>(u.clone())
        {
            usage = Usage::from(parsed);
        }

        let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) else {
            continue;
        };
        let delta = &choice["delta"];

        for field in ["reasoning_content", "reasoning", "reasoning_text"] {
            if let Some(r) = delta.get(field).and_then(|v| v.as_str()) {
                let fresh = if compat.reasoning_deltas_may_be_cumulative
                    && !reasoning.is_empty()
                    && r.len() > reasoning.len()
                    && r.starts_with(reasoning.as_str())
                {
                    r[reasoning.len()..].to_string()
                } else if compat.reasoning_deltas_may_be_cumulative && r == reasoning.as_str() {
                    String::new()
                } else {
                    r.to_string()
                };
                let mut emitted = String::new();
                reasoning_filter.feed(&fresh, &mut emitted);
                if !emitted.is_empty() {
                    reasoning.push_str(&emitted);
                    if !sink.block(text_block(BlockKind::Reasoning, &emitted), false) {
                        interrupted = true;
                    }
                }
                break;
            }
        }
        if interrupted {
            break;
        }

        if let Some(t) = delta.get("content").and_then(|v| v.as_str()) {
            let mut emitted = String::new();
            content_filter.feed(t, &mut emitted);
            if !emitted.is_empty() {
                let structured = delta
                    .get("tool_calls")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| !a.is_empty());
                match scanner.as_mut() {
                    Some(sc) => {
                        for ev in sc.feed(&emitted) {
                            match ev {
                                ScanEvent::Text(t) => {
                                    content.push_str(&t);
                                    if !sink.block(text_block(BlockKind::Assistant, &t), false) {
                                        interrupted = true;
                                    }
                                }
                                ScanEvent::Thinking(t) => {
                                    reasoning.push_str(&t);
                                    if !sink.block(text_block(BlockKind::Reasoning, &t), false) {
                                        interrupted = true;
                                    }
                                }
                                ScanEvent::ToolCall { name, arguments } => {
                                    if !structured {
                                        healed_calls.push(ToolCall::new(
                                            format!("call_healed_{}", healed_calls.len()),
                                            name,
                                            arguments,
                                        ));
                                    }
                                }
                            }
                            if interrupted {
                                break;
                            }
                        }
                    }
                    None => {
                        content.push_str(&emitted);
                        if !sink.block(text_block(BlockKind::Assistant, &emitted), false) {
                            interrupted = true;
                        }
                    }
                }
                if interrupted {
                    break;
                }
            }
        }

        tool_calls.feed(delta);
        if let Some(f) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            finish_reason = Some(f.to_string());
        }
    }

    if !interrupted && let Some(sc) = scanner.as_mut() {
        for ev in sc.flush() {
            match ev {
                ScanEvent::Text(t) => {
                    content.push_str(&t);
                    if !sink.block(text_block(BlockKind::Assistant, &t), false) {
                        interrupted = true;
                    }
                }
                ScanEvent::Thinking(t) => {
                    reasoning.push_str(&t);
                    if !sink.block(text_block(BlockKind::Reasoning, &t), false) {
                        interrupted = true;
                    }
                }
                ScanEvent::ToolCall { name, arguments } => {
                    healed_calls.push(ToolCall::new(
                        format!("call_healed_{}", healed_calls.len()),
                        name,
                        arguments,
                    ));
                }
            }
            if interrupted {
                break;
            }
        }
    }

    if !interrupted {
        let mut tail = String::new();
        content_filter.flush(&mut tail);
        if !tail.is_empty() {
            content.push_str(&tail);
            if !sink.block(text_block(BlockKind::Assistant, &tail), false) {
                interrupted = true;
            }
        }
        let mut rtail = String::new();
        reasoning_filter.flush(&mut rtail);
        if !rtail.is_empty() {
            reasoning.push_str(&rtail);
            if !sink.block(text_block(BlockKind::Reasoning, &rtail), false) {
                interrupted = true;
            }
        }
    }

    if !interrupted
        && compat.empty_length_finish_is_context_error
        && content.is_empty()
        && reasoning.is_empty()
        && finish_reason.as_deref() == Some("length")
    {
        anyhow::bail!("the model stopped on `length` with no output at all");
    }

    let mut calls = tool_calls.finish();
    let healed = !healed_calls.is_empty();
    calls.extend(healed_calls);
    let stop_reason = if interrupted {
        StopReason::Interrupted
    } else if healed || !calls.is_empty() {
        StopReason::ToolCalls
    } else if compat.supports_finish_reason {
        parse_finish_reason(finish_reason)
    } else {
        StopReason::Stop
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
        stop_reason,
        usage,
        reasoning: (!reasoning.is_empty()).then_some(reasoning),
    })
}

fn is_timeout(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::TimedOut
}

pub(crate) fn text_block(kind: BlockKind, text: &str) -> NewBlock {
    NewBlock {
        kind,
        content: Some(text.to_string()),
        ..NewBlock::default()
    }
}

pub(crate) fn assistant_block(text: &str, usage: &Usage) -> NewBlock {
    NewBlock {
        kind: BlockKind::Assistant,
        content: Some(text.to_string()),
        usage_total: Some(usage.total_tokens),
        usage_prompt: Some(usage.prompt_tokens),
        usage_completion: Some(usage.completion_tokens),
        usage_cached: Some(usage.cached_tokens.unwrap_or(0)),
        usage_cache_write: Some(usage.cache_write_tokens.unwrap_or(0)),
        usage_reasoning: Some(usage.reasoning_tokens.unwrap_or(0)),
        ..NewBlock::default()
    }
}

pub(crate) fn tool_block(call: &ToolCall, first: bool) -> NewBlock {
    NewBlock {
        kind: BlockKind::ToolRequest,
        call_id: Some(call.id.clone()),
        name: Some(call.name().to_string()),
        args: Some(call.function.arguments.clone()),
        intent: Some(call.intent()),
        first: Some(first),
        ..NewBlock::default()
    }
}
