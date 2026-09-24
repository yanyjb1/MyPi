//! AI client — mirrors pi's packages/ai/src/api/openai-completions.ts.
//!
//! Its single job: translate the Context into an HTTP request for an
//! OpenAI-compatible endpoint and translate the response (or SSE stream)
//! back into an AssistantMessage. The loop layer never touches HTTP
//! details — that is the point of pi's "ai layer / agent layer" split.

use anyhow::{Context as _, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ai::types::{AssistantMessage, Context, StopReason, Usage};

// Connection details for one provider.
#[derive(Debug, Clone)]
pub struct Client {
    // Like https://host/v1 — requests go to {base_url}/chat/completions
    base_url: String,
    api_key: String,
    model: String,
    http: ureq::Agent,
}

// Non-streaming response body. Only the fields we need are parsed; serde leniency skips the rest.
#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ApiMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiMessage {
    #[serde(default)]
    content: Option<String>,
    // Reasoning-content extension used by thinking models (Qwen et al.).
    #[serde(default)]
    reasoning_content: Option<String>,
    // Tool calls requested this round.
    //
    // Wire name is snake_case, our internal field is `tool_calls`, so no
    // rename needed. A hypothetical camelCase gateway would add its own
    // rename attribute.
    #[serde(default)]
    tool_calls: Vec<crate::ai::types::ToolCall>,
}

// Wire shape of usage (with nested _details), kept separate from the
// internal types::Usage: the wire layer absorbs per-gateway shape
// differences, the internal type stays clean.
#[derive(Debug, Deserialize)]
struct ApiUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    #[serde(default)]
    completion_tokens_details: Option<CompletionDetails>,
    #[serde(default)]
    prompt_tokens_details: Option<PromptDetails>,
}

#[derive(Debug, Deserialize)]
struct CompletionDetails {
    #[serde(default)]
    reasoning_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct PromptDetails {
    #[serde(default)]
    cached_tokens: Option<u64>,
}

impl From<ApiUsage> for Usage {
    fn from(u: ApiUsage) -> Self {
        Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
            reasoning_tokens: u.completion_tokens_details.and_then(|d| d.reasoning_tokens),
            cached_tokens: u.prompt_tokens_details.and_then(|d| d.cached_tokens),
        }
    }
}

// Request body. `messages` is a Value because we inject the
// already-serialized Message array directly, skipping an intermediate
// structure.
#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a Value,
    max_tokens: u32,
    stream: bool,
    // Tool manuals. The whole field is omitted when empty — some strict
    // gateways reject `"tools": []`, and omission keeps a no-tools
    // request byte-identical to the historical shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [crate::ai::types::ToolDef]>,
    // When streaming, ask for a separate usage block at the end of the
    // stream (OpenAI convention); serializes as null for non-streaming,
    // which compatible layers ignore.
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

// Accumulator for streamed tool calls.
//
// The most counter-intuitive part of the streaming protocol: **a single
// tool call arrives as several fragments**. `index` is the pairing key,
// not an array position:
//
// ```text
// {"index":0,"id":"call_abc","function":{"name":"read_file","arguments":""}}
// {"index":0,"function":{"arguments":"{\"pa"}}
// {"index":0,"function":{"arguments":"th\":\"a\"}"}}
// ```
//
// Merge by `index`, concatenating `arguments` in order.
// Two classic bugs, both subtle:
// - Treating `index` as an array position and pushing: fine with one
//   call, chaos with parallel calls.
// - Overwriting the whole entry per fragment: only half a JSON survives,
//   and the failure looks like the model sent bad arguments.
//
// A standalone structure so the stitching logic is testable offline.
#[derive(Default)]
struct ToolCallAccum {
    // index -> call currently being assembled
    pending: Vec<(u64, crate::ai::types::ToolCall)>,
}

impl ToolCallAccum {
    // Feed one delta. A missing `index` is treated as 0 (single-call leniency).
    fn feed(&mut self, delta: &Value) {
        let Some(calls) = delta.get("tool_calls").and_then(|v| v.as_array()) else {
            return;
        };
        for raw in calls {
            let index = raw.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            let entry = match self.pending.iter_mut().find(|(i, _)| *i == index) {
                Some((_, call)) => call,
                None => {
                    // A new call. id/name usually arrive in the first
                    // fragment, but do not assume it — some gateways put
                    // the id in the second one.
                    self.pending.push((
                        index,
                        crate::ai::types::ToolCall::new(
                            String::new(),
                            String::new(),
                            String::new(),
                        ),
                    ));
                    &mut self.pending.last_mut().expect("just pushed").1
                }
            };
            if let Some(id) = raw.get("id").and_then(|v| v.as_str())
                && !id.is_empty()
            {
                // Gateways often **repeat id/name verbatim** in
                // successive deltas; overwrite rather than append, or the
                // id becomes call_xxxcall_xxx and the name
                // editeditedit... Genuinely split ids are rare, and
                // append semantics belong to arguments alone.
                if entry.id.is_empty() || entry.id != id {
                    entry.id = id.to_string();
                }
            }
            if let Some(f) = raw.get("function") {
                if let Some(n) = f.get("name").and_then(|v| v.as_str())
                    && !n.is_empty()
                {
                    // Same for name: overwrite, never append (observed
                    // "editedit..." x100 in live e2e). Append semantics
                    // belong to arguments only.
                    if entry.function.name.is_empty() || entry.function.name != n {
                        entry.function.name = n.to_string();
                    }
                }
                // Arguments **concatenate**, they do not replace. Overwriting here is the classic bug.
                if let Some(a) = f.get("arguments").and_then(|v| v.as_str()) {
                    entry.function.arguments.push_str(a);
                }
            }
        }
    }

    // Finish: return calls ordered by index — preserving the order the
    // model gave.
    fn finish(self) -> Vec<crate::ai::types::ToolCall> {
        let mut v = self.pending;
        v.sort_by_key(|(i, _)| *i);
        v.into_iter().map(|(_, c)| c).collect()
    }
}

impl Client {
    // Current model id.
    pub fn model(&self) -> &str {
        &self.model
    }

    // Switch model within the session (endpoint and key switch along,
    // different models may belong to different providers). Not
    // persisted — a restart returns to the configured default.
    pub fn switch_model(&mut self, base_url: &str, api_key: &str, model: &str) {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self.api_key = api_key.to_string();
        self.model = model.to_string();
    }

    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            http: Self::agent(),
        }
    }

    // The shared HTTP agent.
    //
    // Timeouts are set **only** where they cannot hurt a long generation:
    //  * connect — a black-holed gateway fails in 30 s instead of hanging;
    //  * receive-response — headers must arrive within 120 s, which also
    //    bounds the "gateway accepted but never answered" case that used to
    //    block the turn thread forever (the interrupt flag is only polled
    //    from `on_delta`, which never runs while the read is stuck).
    // There is deliberately **no** body/global timeout: a reply may
    // legitimately stream for minutes, and a total-body deadline would cut it
    // off mid-sentence. A stall *after* the headers therefore still blocks the
    // turn until the socket dies — accepted, documented tradeoff.
    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(120)))
            .build()
            .into()
    }

    // Full chat/completions URL.
    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    // Build the request body.
    //
    // `messages` must be materialized by the caller and passed in — the
    // shape the borrow checker forces here: the function cannot build a
    // temporary `Value` and return a body borrowing it. `tools`
    // likewise borrows `ctx`; `None` omits the field entirely.
    fn build_body<'a>(
        &'a self,
        max_tokens: u32,
        stream: bool,
        messages: &'a Value,
        tools: &'a [crate::ai::types::ToolDef],
    ) -> ChatRequest<'a> {
        ChatRequest {
            model: &self.model,
            messages,
            max_tokens,
            stream,
            tools: (!tools.is_empty()).then_some(tools),
            stream_options: if stream {
                Some(StreamOptions {
                    include_usage: true,
                })
            } else {
                None
            },
        }
    }

    // Send a POST with the auth header.
    //
    // Rationale for extracting: `complete` and `stream` used to
    // duplicate the URL join and `Bearer` header **character for
    // character**. Adding a `User-Agent`, changing auth, or appending a
    // query parameter would then be fixed in one place and missed in
    // the other — "non-streaming works, streaming does not", the
    // hardest class of inconsistency to trace.
    fn post(&self, body: &ChatRequest<'_>) -> anyhow::Result<ureq::http::Response<ureq::Body>> {
        self.http
            .post(self.endpoint())
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(body)
            .context("HTTP request failed")
    }

    // Non-streaming: send the whole conversation, get one complete reply.
    //
    // The TUI currently uses `stream` only. This method stays compiled
    // because it is half of the protocol's full capability (pi's
    // packages/ai provides both) — future "summarize"/"title" style
    // requests that do not need per-character display are simpler
    // non-streaming, with no interrupt callback in the way.
    //
    // It shares `build_body` / `post` with `stream`, so the two cannot
    // drift apart.
    pub fn complete(&self, ctx: &Context, max_tokens: u32) -> anyhow::Result<AssistantMessage> {
        let messages =
            serde_json::to_value(&ctx.messages).expect("Message serialization is infallible");
        let body = self.build_body(max_tokens, false, &messages, &ctx.tools);
        let resp: ChatResponse = self
            .post(&body)?
            .body_mut()
            .read_json()
            .context("response is not valid JSON")?;

        let choice = resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("response has no choices"))?;

        Ok(AssistantMessage {
            content: choice.message.content.unwrap_or_default(),
            tool_calls: choice.message.tool_calls,
            stop_reason: parse_finish_reason(choice.finish_reason),
            usage: resp.usage.map(Usage::from).unwrap_or_default(),
            reasoning: choice.message.reasoning_content,
        })
    }

    // Streaming: send the whole conversation, invoke the callback per SSE chunk, return the assembled reply.
    //
    // SSE format:
    //   data: {"choices":[{"delta":{"content":"hi"}}]}   <- one increment
    //   data: [DONE]                                      <- end marker
    //
    // `on_delta` fires per content chunk (the REPL renders typewriter
    // output from it).
    //
    // **Interrupt**: returning `false` from `on_delta` stops reading and
    // disconnects immediately. A real disconnect — dropping the reader
    // closes the socket and the upstream stops generating, so no tokens
    // burn for nothing.
    pub fn stream(
        &self,
        ctx: &Context,
        max_tokens: u32,
        mut on_delta: impl FnMut(&str) -> bool,
        mut on_reasoning: impl FnMut(&str),
    ) -> anyhow::Result<AssistantMessage> {
        let messages =
            serde_json::to_value(&ctx.messages).expect("Message serialization is infallible");
        let body = self.build_body(max_tokens, true, &messages, &ctx.tools);

        let resp = self.post(&body)?;

        // into_reader() yields a byte-level Read; wrap in BufReader for line-wise framing
        let reader = std::io::BufReader::new(resp.into_body().into_reader());

        let mut content = String::new();
        let mut reasoning = String::new();
        let mut finish_reason: Option<String> = None;
        let mut usage = Usage::default();
        let mut interrupted = false;
        let mut tool_calls = ToolCallAccum::default();

        for line in std::io::BufRead::lines(reader) {
            let line = line.context("stream read interrupted")?;
            // Blank lines separate SSE events; only `data:` lines carry payloads
            let Some(payload) = line.strip_prefix("data: ") else {
                continue;
            };
            if payload == "[DONE]" {
                break;
            }

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

            // Reasoning streams through to upstream (UI displays live)
            // and accumulates into the final message. Three field names
            // are accepted (pi's list): reasoning_content / reasoning /
            // reasoning_text
            for field in ["reasoning_content", "reasoning", "reasoning_text"] {
                if let Some(r) = delta.get(field).and_then(|v| v.as_str()) {
                    reasoning.push_str(r);
                    on_reasoning(r);
                    break; // when multiple fields carry the same value, take the first only
                }
            }
            if let Some(t) = delta.get("content").and_then(|v| v.as_str()) {
                content.push_str(t);
                if !on_delta(t) {
                    // Upstream requested interrupt: break the loop; dropping the reader closes the connection
                    interrupted = true;
                    break;
                }
            }
            // Tool-call fragments (see the ToolCallAccum docs)
            tool_calls.feed(delta);
            if let Some(f) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                finish_reason = Some(f.to_string());
            }
        }

        let tool_calls = tool_calls.finish();
        Ok(AssistantMessage {
            content,
            tool_calls,
            stop_reason: if interrupted {
                StopReason::Interrupted
            } else {
                parse_finish_reason(finish_reason)
            },
            usage,
            reasoning: if reasoning.is_empty() {
                None
            } else {
                Some(reasoning)
            },
        })
    }
}

fn parse_finish_reason(reason: Option<String>) -> StopReason {
    match reason.as_deref() {
        Some("length") => StopReason::Length,
        // The model requested tools — the loop's continue signal
        Some("tool_calls") | Some("function_call") => StopReason::ToolCalls,
        // "stop" is the norm; unknown values are leniently treated as stop (a common compatibility-layer flaw)
        _ => StopReason::Stop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn client() -> Client {
        Client::new("http://example.test/v1/", "k", "m")
    }

    #[test]
    fn base_url_trailing_slash_is_trimmed() {
        // A fat-fingered trailing slash in YAML is common; must not become //chat/completions
        assert_eq!(
            client().endpoint(),
            "http://example.test/v1/chat/completions"
        );
    }

    #[test]
    fn streaming_body_requests_a_usage_block() {
        // The only difference between non-streaming and streaming is
        // stream + stream_options. This behavior once had no test — and
        // with `complete` and `stream` each building a body, this is
        // where mistakes happen (forgetting include_usage on the
        // streaming side silently zeroes the cost bar).
        let c = client();
        let msgs = json!([]);
        let tools: Vec<crate::ai::types::ToolDef> = vec![];
        let stream = c.build_body(100, true, &msgs, &tools);
        assert!(stream.stream);
        assert_eq!(
            stream.stream_options.as_ref().map(|o| o.include_usage),
            Some(true),
            "streaming must request the usage block or pricing gets no data"
        );

        let plain = c.build_body(100, false, &msgs, &tools);
        assert!(!plain.stream);
        assert!(
            plain.stream_options.is_none(),
            "non-streaming must not send stream_options"
        );
        // Without tools the whole field must be omitted, not an empty array
        assert!(serde_json::to_value(&plain).unwrap().get("tools").is_none());
    }

    #[test]
    fn tools_field_is_present_only_when_defined() {
        let c = client();
        let msgs = json!([]);
        let tools = vec![crate::ai::types::ToolDef::function(
            "read_file",
            "read a file",
            json!({"type": "object"}),
        )];
        let body = c.build_body(100, false, &msgs, &tools);
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["tools"][0]["type"], "function");
        assert_eq!(v["tools"][0]["function"]["name"], "read_file");
    }

    #[test]
    fn finish_reason_maps_length_only() {
        assert_eq!(
            parse_finish_reason(Some("length".into())),
            StopReason::Length
        );
        assert_eq!(
            parse_finish_reason(Some("tool_calls".into())),
            StopReason::ToolCalls
        );
        assert_eq!(parse_finish_reason(Some("stop".into())), StopReason::Stop);
        // Unknown values leniently map to stop (a common compatibility-layer flaw)
        assert_eq!(parse_finish_reason(Some("weird".into())), StopReason::Stop);
        assert_eq!(parse_finish_reason(None), StopReason::Stop);
    }

    #[test]
    fn fragmented_tool_call_delta_is_stitched_back_together() {
        // The easiest place to get streaming tool calls wrong; fragments
        // captured from a real gateway: id+name in the first, arguments
        // split across three.
        let mut acc = ToolCallAccum::default();
        // Arguments split in three and stitched back to {"path":"a"} — verifies concat, not replace
        let pieces = [
            r#"{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"read_file","arguments":""}}]}"#,
            r#"{"tool_calls":[{"index":0,"function":{"arguments":"{\"pa"}}]}"#,
            r#"{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a\"}"}}]}"#,
        ];
        for piece in pieces {
            let v: Value = serde_json::from_str(piece).unwrap();
            acc.feed(&v);
        }
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_abc");
        assert_eq!(calls[0].name(), "read_file");
        assert_eq!(calls[0].function.arguments, r#"{"path":"a"}"#);
    }

    #[test]
    fn parallel_tool_calls_keep_their_order_by_index() {
        // Two parallel calls interleaving. Index-ordered merge must
        // restore order 0, 1 — reversing it would pair results to the
        // wrong tool_call_id downstream.
        let mut acc = ToolCallAccum::default();
        for piece in [
            r#"{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"write_file","arguments":""}}]}"#,
            r#"{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read_file","arguments":""}}]}"#,
            r#"{"tool_calls":[{"index":1,"function":{"arguments":"{}"}}]}"#,
            r#"{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]}"#,
        ] {
            let v: Value = serde_json::from_str(piece).unwrap();
            acc.feed(&v);
        }
        let calls = acc.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name(), "read_file");
        assert_eq!(calls[1].name(), "write_file");
    }

    #[test]
    fn missing_index_defaults_to_zero() {
        // Some compatibility layers omit index. Treating it as 0 is the
        // most lenient choice; an error would only confuse users, and
        // leniency cannot misbehave in the single-call case.
        let mut acc = ToolCallAccum::default();
        let v: Value = serde_json::from_str(
            r#"{"tool_calls":[{"id":"c1","function":{"name":"f","arguments":"{}"}}]}"#,
        )
        .unwrap();
        acc.feed(&v);
        assert_eq!(acc.finish().len(), 1);
    }

    #[test]
    fn non_tool_deltas_are_ignored_by_the_accumulator() {
        // Content/reasoning chunks carry no tool_calls; feeding them must be a no-op
        let mut acc = ToolCallAccum::default();
        for piece in [
            r#"{"content":"hi"}"#,
            r#"{"reasoning_content":"hmm"}"#,
            r#"{}"#,
        ] {
            let v: Value = serde_json::from_str(piece).unwrap();
            acc.feed(&v);
        }
        assert!(acc.finish().is_empty());
    }
}
