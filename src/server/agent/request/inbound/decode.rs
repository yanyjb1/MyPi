//! The inbound decode: `data:` lines become one `AssistantMessage`. Text and reasoning stream out as
//! increments, tool calls accumulate until the end. This layer knows no HTTP and no retry.

use anyhow::Context as _;
use serde_json::Value;

use crate::server::agent::request::content::AnswerSink;
use crate::server::agent::request::inbound::markup::TemplateMarkupFilter;
use crate::server::agent::request::inbound::response::{ApiUsage, ToolCallAccum};
use crate::server::agent::request::message::{AssistantMessage, StopReason, Usage};
use crate::server::agent::request::transport::client::{Client, HttpFailure};

/// Streaming decode: text and reasoning go to `sink` as they arrive (`false` = stop, drop the reader),
/// tool calls accumulate. An idle read is not a failure: each read has a 1 s timeout to poll the flag.
pub(crate) fn streamed(
    mut reader: Box<dyn std::io::BufRead>,
    client: &Client,
    cancel: &std::sync::atomic::AtomicBool,
    sink: &mut dyn AnswerSink,
) -> anyhow::Result<AssistantMessage> {
    let compat = &client.compat;
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut finish_reason: Option<String> = None;
        // Marker filters and in-band parsing: without the scanner the tools inside a text protocol
        // would never run. Ids for the calls found there are generated here, in order.
        let mut scanner = crate::server::agent::request::inbound::inband::scanner_for(compat);
        // Calls parsed out of the text: their ids are generated here, in order, since hosts give none.
        let mut healed_calls: Vec<crate::server::agent::request::message::ToolCall> = Vec::new();
        let mut content_filter = TemplateMarkupFilter::for_compat(compat);
        let mut reasoning_filter = TemplateMarkupFilter::for_compat(compat);
        let mut usage = Usage::default();
        let mut interrupted = false;
        let mut tool_calls = ToolCallAccum::default();

        // Reading a line must tolerate idleness: `recv_body` times out per read, and that is the chance
        // to poll the interrupt flag. `read_until`, not `lines()` — the latter ends the stream on it.
        let mut pending: Vec<u8> = Vec::new();
        // First-event timeout: unlimited by default; `stream_first_event_timeout_ms` opens it per host.
        let started = std::time::Instant::now();
        let first_event_budget = compat
            .stream_first_event_timeout_ms
            .filter(|ms| *ms > 0)
            .map(std::time::Duration::from_millis);
        let mut got_first = false;
        // Idle limit between events, after the first: 0 = unlimited.
        let idle_budget = compat
            .stream_idle_timeout_ms
            .filter(|ms| *ms > 0)
            .map(std::time::Duration::from_millis);
        let mut last_event = std::time::Instant::now();
        loop {
            pending.clear();
            match std::io::BufRead::read_until(&mut reader, b'\n', &mut pending) {
                Ok(0) => {
                    // EOF is not a clean end: a killed worker looks the same. Without the flag check the
                    // interrupted round would be recorded as a normal stop.
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        interrupted = true;
                    }
                    break;
                }
                Ok(_) => {}
                Err(e) if Client::is_body_read_timeout(&e) => {
                    // Idle: one chance for the interrupt flag; already set = the user pressed Esc.
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        interrupted = true;
                        break;
                    }
                    // No first event within the host's own limit: stuck, not slow.
                    if !got_first
                        && let Some(budget) = first_event_budget
                        && started.elapsed() > budget
                    {
                        anyhow::bail!(
                            "no first event from upstream within {:?}; treating it as stuck ({}'s first-event timeout — \
                             do not set this for a host that is slow to start locally)",
                            budget,
                            client.model,
                        );
                    }
                    // Already streaming: stalled, not slow.
                    if got_first
                        && let Some(budget) = idle_budget
                        && last_event.elapsed() > budget
                    {
                        anyhow::bail!(
                            "no new event from upstream within {:?}; treating it as hung ({}'s inter-event idle timeout)",
                            budget,
                            client.model,
                        );
                    }
                    continue;
                }
                Err(e) => {
                    // Interrupt: EOF after a killed worker is expected control flow, not an error.
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        interrupted = true;
                        break;
                    }
                    return Err(e).context("stream read interrupted");
                }
            }
            let line = String::from_utf8_lossy(&pending);
            let line = line.strip_suffix('\n').unwrap_or(&line);
            let line = line.strip_suffix('\r').unwrap_or(line);
            // The worker's death report is not a bad line: it carries a status error (`__HTTP__`) so
            // retry and strict-downgrade decisions match the direct path, and a dead slot is rebuilt.
            if let Some(err) = line.strip_prefix("__WORKER_ERROR__") {
                // The worker died on its own (an interrupt was handled at EOF): rebuild the slot here.
                if !err.starts_with("__HTTP__")
                    && !cancel.load(std::sync::atomic::Ordering::Relaxed)
                    && (|| {
                        // No worker in the slot any more: the round falls back to a plain death error.
                        let slot = client.worker.lock().ok()?;
                        let inner = slot.clone()?;
                        crate::server::agent::request::transport::worker::HttpWorker::kill_and_replace(&inner).ok()
                    })()
                    .is_some()
                {
                    anyhow::bail!("HTTP worker crashed; a fresh one took over (retry)");
                }
                if let Some(rest) = err.strip_prefix("__HTTP__") {
                    let (status, body) = match rest.split_once('\n') {
                        Some((s, b)) => (s, b),
                        None => (rest, ""),
                    };
                    if let Ok(status) = status.parse::<u16>() {
                        anyhow::bail!(anyhow::Error::new(HttpFailure {
                            status,
                            body: body.to_string(),
                        }));
                    }
                }
                anyhow::bail!("HTTP worker died: {err}");
            }
            let Some(payload) = line.strip_prefix("data: ") else {
                continue;
            };
            if payload == "[DONE]" {
                break;
            }
            got_first = true;
            last_event = std::time::Instant::now();

            let chunk: Value = serde_json::from_str(payload)
                .with_context(|| format!("malformed stream chunk: {payload}"))?;

            // Compatible layers may send usage in its own chunk (the stream_options.include_usage contract)
            if let Some(u) = chunk.get("usage").filter(|v| !v.is_null())
                && let Ok(parsed) = serde_json::from_value::<ApiUsage>(u.clone())
            {
                usage = Usage::from(parsed);
            }

            let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) else {
                continue; // usage chunk has an empty choices array
            };
            let delta = &choice["delta"];

            // Reasoning streams through and accumulates into the final message. Three field names are
            // accepted: reasoning_content / reasoning / reasoning_text.
            for field in ["reasoning_content", "reasoning", "reasoning_text"] {
                if let Some(r) = delta.get(field).and_then(|v| v.as_str()) {
                    // Some hosts resend the whole reasoning text each chunk instead of a delta: if the new
                    // text starts with everything so far, the increment is its tail.
                    let fresh = if client.compat.reasoning_deltas_may_be_cumulative
                        && !reasoning.is_empty()
                        && r.len() > reasoning.len()
                        && r.starts_with(reasoning.as_str())
                    {
                        r[reasoning.len()..].to_string()
                    } else if client.compat.reasoning_deltas_may_be_cumulative
                        && r == reasoning.as_str()
                    {
                        String::new() // a pure repeat: nothing new
                    } else {
                        r.to_string()
                    };
                    let mut emitted = String::new();
                    reasoning_filter.feed(&fresh, &mut emitted);
                    reasoning.push_str(&emitted);
                    if !emitted.is_empty() && !sink.reasoning(&emitted) {
                        // Interrupted while thinking: same as the content
                        // path — break, drop the reader, the socket closes.
                        interrupted = true;
                    }
                    break; // when multiple fields carry the same value, take the first only
                }
            }
            if interrupted {
                break;
            }
            if let Some(t) = delta.get("content").and_then(|v| v.as_str()) {
                let mut emitted = String::new();
                content_filter.feed(t, &mut emitted);
                // A chunk can carry `content: ""` next to tool-call fragments. Skip only the text
                // handling — `continue` here would drop the fragments and the round would end empty.
                if !emitted.is_empty() {
                    // With structured `tool_calls` present, the copy inside the text is a duplicate:
                    // show it, never synthesize a second call.
                    let structured = delta
                        .get("tool_calls")
                        .and_then(|v| v.as_array())
                        .is_some_and(|a| !a.is_empty());
                    match scanner.as_mut() {
                        Some(sc) => {
                            for ev in sc.feed(&emitted) {
                                match ev {
                                    crate::server::agent::request::inbound::inband::ScanEvent::Text(t) => {
                                        content.push_str(&t);
                                        if !sink.content(&t) {
                                            interrupted = true;
                                        }
                                    }
                                    crate::server::agent::request::inbound::inband::ScanEvent::Thinking(t) => {
                                        reasoning.push_str(&t);
                                        if !sink.reasoning(&t) {
                                            interrupted = true;
                                        }
                                    }
                                    crate::server::agent::request::inbound::inband::ScanEvent::ToolCall {
                                        name,
                                        arguments,
                                    } => {
                                        if !structured {
                                            healed_calls.push(crate::server::agent::request::message::ToolCall {
                                                id: format!("call_healed_{}", healed_calls.len()),
                                                kind: "function".into(),
                                                function: crate::server::agent::request::message::FunctionCall {
                                                    name,
                                                    arguments,
                                                },
                                            });
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
                            if !sink.content(&emitted) {
                                interrupted = true;
                            }
                        }
                    }
                    if interrupted {
                        break;
                    }
                }
            }
            // Tool-call fragments (see the ToolCallAccum docs)
            tool_calls.feed(delta);
            if let Some(f) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                finish_reason = Some(f.to_string());
            }
        }

        // The stream ended: the scanner must hand back what it held, or text is lost.
        if !interrupted
            && let Some(sc) = scanner.as_mut()
        {
            for ev in sc.flush() {
                match ev {
                    crate::server::agent::request::inbound::inband::ScanEvent::Text(t) => {
                        content.push_str(&t);
                        sink.content(&t);
                    }
                    crate::server::agent::request::inbound::inband::ScanEvent::Thinking(t) => {
                        reasoning.push_str(&t);
                        sink.reasoning(&t);
                    }
                    crate::server::agent::request::inbound::inband::ScanEvent::ToolCall { name, arguments } => {
                        healed_calls.push(crate::server::agent::request::message::ToolCall {
                            id: format!("call_healed_{}", healed_calls.len()),
                            kind: "function".into(),
                            function: crate::server::agent::request::message::FunctionCall { name, arguments },
                        });
                    }
                }
            }
        }

        // Whatever the markup filters held back was a false alarm (no marker
        // ever completed) — hand it over rather than swallowing user text.
        if !interrupted {
            let mut tail = String::new();
            content_filter.flush(&mut tail);
            if !tail.is_empty() {
                content.push_str(&tail);
                sink.content(&tail);
            }
            let mut rtail = String::new();
            reasoning_filter.flush(&mut rtail);
            if !rtail.is_empty() {
                reasoning.push_str(&rtail);
                sink.reasoning(&rtail);
            }
        }

        // A full context window can arrive as a perfectly normal response with `finish_reason: "length"`
        // and no text. Naming the real cause saves a hunt for a model problem.
        if !interrupted
            && compat.empty_length_finish_is_context_error
            && content.is_empty()
            && reasoning.is_empty()
            && finish_reason.as_deref() == Some("length")
        {
            anyhow::bail!(
                "the model stopped on `length` with no output at all: the context window is most likely full (this host says so with an empty reply)"
            );
        }

        let mut tool_calls = tool_calls.finish();
        let healed = healed_calls.is_empty() == false;
        tool_calls.extend(healed_calls);
        let stop_reason = if interrupted {
            StopReason::Interrupted
        } else if healed {
            // A call parsed out of the text usually arrives with `finish_reason: "stop"`: promote it.
            StopReason::ToolCalls
        } else {
            client.stop_reason(finish_reason, !tool_calls.is_empty())
        };
        Ok(AssistantMessage {
            content,
            tool_calls,
            stop_reason,
            usage,
            reasoning: if reasoning.is_empty() {
                None
            } else {
                Some(reasoning)
            },
        })
}
