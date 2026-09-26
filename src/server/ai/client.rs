//! AI client — mirrors pi's packages/ai/src/api/openai-completions.ts.
//!
//! Its single job: translate the Context into an HTTP request for an
//! OpenAI-compatible endpoint and translate the response (or SSE stream)
//! back into an AssistantMessage. The loop layer never touches HTTP
//! details — that is the point of pi's "ai layer / agent layer" split.

use anyhow::{Context as _, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::server::ai::types::{AssistantMessage, Context, StopReason, Usage};

// Connection details for one provider.
#[derive(Debug, Clone)]
pub struct Client {
    // Like https://host/v1 — requests go to {base_url}/chat/completions
    base_url: String,
    api_key: String,
    model: String,
    http: ureq::Agent,
    retry: RetryPolicy,
}

/// The one protocol this client speaks. Stored with every round (see
/// `store::RoundMeta`) so replay is a data question, not a guess.
pub const PROTOCOL: &str = "openai-chat-completions";

/// Retry policy for one HTTP request — the common defaults, no cleverness.
///
/// Three attempts total (the first try plus two retries), 400 ms then 800 ms
/// with ±25% jitter, capped at 8 s. Retries are logged to the in-memory log
/// (`server::log`), never persisted: a retry is something *we* did, not
/// something the model saw.
///
/// Streaming requests are retried **only while nothing has been emitted** —
/// once text has reached the user, a retry would duplicate it (the session
/// keeps the partial reply instead; see `SessionState::finalize_round`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total tries, including the first.
    pub attempts: u32,
    pub base_ms: u64,
    pub factor: u32,
    pub max_ms: u64,
    /// Jitter the backoff (±25%) so several sessions do not retry in lockstep.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 3,
            base_ms: 400,
            factor: 2,
            max_ms: 8_000,
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// Backoff before try number `attempt + 1` (`attempt` = tries already made).
    fn delay_ms(&self, attempt: u32) -> u64 {
        let step = self
            .factor
            .saturating_pow(attempt.saturating_sub(1))
            .max(1) as u64;
        let raw = self.base_ms.saturating_mul(step).min(self.max_ms);
        if !self.jitter {
            return raw;
        }
        let spread = (raw / 4).max(1);
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::from(d.subsec_nanos()))
            .unwrap_or(0);
        raw - spread + (seed % (2 * spread + 1))
    }
}

/// Is this failure worth another try?
///
/// Yes for the transient classes (timeouts, socket errors, a dead connection,
/// DNS, 408/409/425/429, 5xx). No for anything that means "the request or the
/// answer is wrong" (bad URI, protocol error, TLS, redirect loop, 4xx other
/// than the ones above): retrying those just burns time and money.
fn retryable(e: &anyhow::Error) -> bool {
    if let Some(u) = e.downcast_ref::<ureq::Error>() {
        return match u {
            ureq::Error::StatusCode(code) => {
                matches!(*code, 408 | 409 | 425 | 429 | 500 | 502 | 503 | 504)
            }
            ureq::Error::Timeout(_)
            | ureq::Error::Io(_)
            | ureq::Error::ConnectionFailed
            | ureq::Error::HostNotFound => true,
            _ => false,
        };
    }
    // Reading a response body reports plain io errors.
    e.downcast_ref::<std::io::Error>().is_some()
}

/// Serialize one request body.
///
/// **The single source of the request's bytes.** The live path (`complete` /
/// `stream`) and the replay path (rebuilding a stored round from the DB) both
/// call this, so a replayed request cannot drift from the one that was sent —
/// which is the whole point of storing the header at all. Everything variable
/// arrives as an argument: nothing is read from local config here.
pub(crate) fn body_json(
    model: &str,
    max_tokens: u32,
    stream: bool,
    messages: &serde_json::Value,
    tools: &serde_json::Value,
) -> serde_json::Value {
    // Tools arrive already serialized: the live path serializes the tool
    // registry, the replay path parses the stored manuals — one shape either
    // way, and an empty roster omits the field entirely (strict gateways reject
    // `"tools": []`).
    let tools = tools
        .as_array()
        .filter(|a| !a.is_empty())
        .map(|_| tools);
    let body = ChatRequest {
        model,
        messages,
        max_tokens,
        stream,
        tools,
        stream_options: if stream {
            Some(StreamOptions {
                include_usage: true,
            })
        } else {
            None
        },
    };
    // Through `Value` (not the struct directly) so live and replay produce
    // identical bytes: one serialization route, not two.
    serde_json::to_value(&body).expect("request body serialization is infallible")
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
    tool_calls: Vec<crate::server::ai::types::ToolCall>,
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
    // request byte-identical to the historical shape. Pre-serialized so the
    // replay path (which only has the stored JSON) produces the same bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a serde_json::Value>,
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
    pending: Vec<(u64, crate::server::ai::types::ToolCall)>,
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
                        crate::server::ai::types::ToolCall::new(
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
    fn finish(self) -> Vec<crate::server::ai::types::ToolCall> {
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

    // Endpoint base as configured (`https://host/v1`, no trailing slash).
    // Recorded with every round so a stored conversation names the exact
    // endpoint it was spoken to (never the key).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    // Wire protocol this client speaks. Recorded per round: a future second
    // protocol must store its own id, so replay never assumes a shape.
    pub fn protocol(&self) -> &'static str {
        PROTOCOL
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
            retry: RetryPolicy::default(),
        }
    }

    /// Replace the retry policy (tests use a tiny backoff; a future config key
    /// would use this too).
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
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

    // Send a POST with the auth header.
    //
    // Rationale for extracting: `complete` and `stream` used to
    // duplicate the URL join and `Bearer` header **character for
    // character**. Adding a `User-Agent`, changing auth, or appending a
    // query parameter would then be fixed in one place and missed in
    // the other — "non-streaming works, streaming does not", the
    // hardest class of inconsistency to trace.
    fn post(&self, body: &serde_json::Value) -> anyhow::Result<ureq::http::Response<ureq::Body>> {
        self.http
            .post(self.endpoint())
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(body)
            .context("HTTP request failed")
    }

    /// Log one failed try and say whether another is allowed.
    ///
    /// Logging lives here (not at the call site) so every retry reads the same
    /// way in the log: "第 2/3 次请求失败：… ；800ms 后重试".
    fn should_retry(&self, n: u32, e: &anyhow::Error) -> bool {
        if n < self.retry.attempts && retryable(e) {
            crate::server::log::warn(
                &self.model,
                format!(
                    "第 {n}/{} 次请求失败：{e:#}；{}ms 后重试",
                    self.retry.attempts,
                    self.retry.delay_ms(n)
                ),
            );
            true
        } else {
            crate::server::log::error(
                &self.model,
                format!(
                    "{e:#}（{}）",
                    if n >= self.retry.attempts {
                        format!("已试 {} 次，放弃", self.retry.attempts)
                    } else {
                        "不可重试的错误".to_string()
                    }
                ),
            );
            false
        }
    }

    /// POST with retries on the retryable failure classes (non-streaming path).
    fn post_retrying(
        &self,
        body: &serde_json::Value,
    ) -> anyhow::Result<ureq::http::Response<ureq::Body>> {
        let mut n = 1;
        loop {
            match self.post(body) {
                Ok(resp) => {
                    if n > 1 {
                        crate::server::log::info(
                            &self.model,
                            format!("第 {n} 次尝试成功"),
                        );
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    if !self.should_retry(n, &e) {
                        return Err(e);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(self.retry.delay_ms(n)));
                    n += 1;
                }
            }
        }
    }

    // Non-streaming: send the whole conversation, get one complete reply.
    //
    // The TUI currently uses `stream` only. This method stays compiled
    // because it is half of the protocol's full capability (pi's
    // packages/ai provides both) — future "summarize"/"title" style
    // requests that do not need per-character display are simpler
    // non-streaming, with no interrupt callback in the way.
    //
    // It shares `body_json` / `post` with `stream`, so the two cannot
    // drift apart.
    pub fn complete(&self, ctx: &Context, max_tokens: u32) -> anyhow::Result<AssistantMessage> {
        let messages =
            serde_json::to_value(&ctx.messages).expect("Message serialization is infallible");
        let tools = serde_json::to_value(&ctx.tools).expect("tool serialization is infallible");
        let body = body_json(&self.model, max_tokens, false, &messages, &tools);
        let resp: ChatResponse = self
            .post_retrying(&body)?
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

    /// Streaming request with the retry shell.
    ///
    /// Retrying a stream is only legal **while nothing has been emitted**: once
    /// a chunk has reached the caller, a retry would replay the beginning of the
    /// reply and the user would see the text twice. Past that point the failure
    /// is real — the session stores the partial reply and the turn ends (that is
    /// what makes a dropped connection non-destructive).
    ///
    /// Every failed try and every give-up goes to the in-memory log.
    pub fn stream(
        &self,
        ctx: &Context,
        max_tokens: u32,
        mut on_delta: impl FnMut(&str) -> bool,
        // Returning false = interrupt: stop reading and disconnect. Same
        // contract as `on_delta`; a thinking-only stream (no content chunks)
        // would otherwise be uninterruptible until the first token.
        mut on_reasoning: impl FnMut(&str) -> bool,
    ) -> anyhow::Result<AssistantMessage> {
        let emitted = std::cell::Cell::new(false);
        let mut n = 1;
        loop {
            let r = self.stream_once(
                ctx,
                max_tokens,
                |d| {
                    emitted.set(true);
                    on_delta(d)
                },
                |r| {
                    emitted.set(true);
                    on_reasoning(r)
                },
            );
            match r {
                Ok(msg) => {
                    if n > 1 {
                        crate::server::log::info(&self.model, format!("第 {n} 次尝试成功"));
                    }
                    return Ok(msg);
                }
                Err(e) => {
                    // Already streaming text: no retry, whatever the cause.
                    let again = !emitted.get() && self.should_retry(n, &e);
                    if !again {
                        return Err(e);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(self.retry.delay_ms(n)));
                    n += 1;
                }
            }
        }
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
    fn stream_once(
        &self,
        ctx: &Context,
        max_tokens: u32,
        mut on_delta: impl FnMut(&str) -> bool,
        mut on_reasoning: impl FnMut(&str) -> bool,
    ) -> anyhow::Result<AssistantMessage> {
        let messages =
            serde_json::to_value(&ctx.messages).expect("Message serialization is infallible");
        let tools = serde_json::to_value(&ctx.tools).expect("tool serialization is infallible");
        let body = body_json(&self.model, max_tokens, true, &messages, &tools);

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
                    if !on_reasoning(r) {
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
    fn the_backoff_grows_then_caps_and_never_returns_zero() {
        // 常用设定：400ms、800ms、上限 8s；抖动不改变区间。
        let p = RetryPolicy {
            jitter: false,
            ..RetryPolicy::default()
        };
        assert_eq!(p.delay_ms(1), 400);
        assert_eq!(p.delay_ms(2), 800);
        assert_eq!(p.delay_ms(3), 1_600);
        assert_eq!(p.delay_ms(9), 8_000, "超过上限就压在上限");
        // 抖动版本落在 ±25% 里，绝不为 0（立刻重试会打爆对端）
        let j = RetryPolicy::default();
        for _ in 0..200 {
            let d = j.delay_ms(1);
            assert!((300..=500).contains(&d), "抖动越界：{d}");
        }
    }

    #[test]
    fn only_transient_failures_are_retryable() {
        let io = anyhow::Error::new(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "断了"))
            .context("stream read interrupted");
        assert!(retryable(&io), "socket 错误必须重试");
        for code in [408, 409, 425, 429, 500, 502, 503, 504] {
            let e = anyhow::Error::new(ureq::Error::StatusCode(code));
            assert!(retryable(&e), "{code} 应当重试");
        }
        for code in [400, 401, 403, 404, 422] {
            let e = anyhow::Error::new(ureq::Error::StatusCode(code));
            assert!(!retryable(&e), "{code} 重试没意义");
        }
        let bad = anyhow::Error::new(ureq::Error::BadUri("nope".into()));
        assert!(!retryable(&bad), "坏 URL 重试多少次都一样");
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
        let tools = json!([]);
        let stream = body_json(c.model(), 100, true, &msgs, &tools);
        assert_eq!(stream["stream"], json!(true));
        assert_eq!(
            stream["stream_options"]["include_usage"],
            json!(true),
            "streaming must request the usage block or pricing gets no data"
        );

        let plain = body_json(c.model(), 100, false, &msgs, &tools);
        assert_eq!(plain["stream"], json!(false));
        assert!(
            plain.get("stream_options").is_none(),
            "non-streaming must not send stream_options"
        );
        // Without tools the whole field must be omitted, not an empty array
        assert!(plain.get("tools").is_none());
    }

    #[test]
    fn tools_field_is_present_only_when_defined() {
        let c = client();
        let msgs = json!([]);
        let tools = json!([{
            "type": "function",
            "function": {"name": "read_file", "description": "read a file",
                         "parameters": {"type": "object"}}
        }]);
        let v = body_json(c.model(), 100, false, &msgs, &tools);
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
