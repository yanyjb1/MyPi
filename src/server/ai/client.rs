//! AI client — mirrors pi's packages/ai/src/api/openai-completions.ts.
//!
//! Its single job: translate the Context into an HTTP request for an
//! OpenAI-compatible endpoint and translate the response (or SSE stream)
//! back into an AssistantMessage. The loop layer never touches HTTP
//! details — that is the point of pi's "ai layer / agent layer" split.

use anyhow::{Context as _, anyhow};
struct ChannelReader { rx: std::sync::mpsc::Receiver<String>, pending: Vec<u8> }
impl std::io::Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pending.is_empty() {
            let line = self.rx.recv().map_err(|_| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "worker closed"))?;
            self.pending.extend_from_slice(line.as_bytes());
            self.pending.push(b'\n');
        }
        let n = out.len().min(self.pending.len());
        out[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}
use serde_json::Value;

use crate::server::ai::types::{AssistantMessage, Context, StopReason, Usage};
use crate::server::ai::assemble::{body_json, messages_to_wire};
use crate::server::ai::stream_patch::TemplateMarkupFilter;
use crate::server::ai::wire::{ApiUsage, ChatResponse, ToolCallAccum, parse_finish_reason};

// Connection details for one provider.
#[derive(Debug, Clone)]
pub struct Client {
    base_url: String,
    api_key: String,
    model: String,
    /// Resolved compat switches for the current endpoint + model.
    ///
    /// Recomputed whenever the endpoint or id changes (`new`, `switch_model`),
    /// then read by `body_json`. Never read from config at request time — same
    /// reason the round row stores the bytes: what went out is data, not a
    /// re-derivation.
    compat: crate::server::ai::compat::Resolved,
    /// The `compat:` block as the user wrote it. Kept alongside the resolved
    /// view because it is the half that is *not* recomputable: detection is a
    /// pure function of (base_url, model), a hand-written override is not, and
    /// the round row has to store what actually shaped the request.
    user_compat: Option<crate::server::ai::compat::Compat>,
    /// 会话标识，喂给 `prompt_cache_session_header`。`None` = 不发那个头。
    ///
    /// 有的宿主靠一个请求头把同一会话的请求粘到同一个后端实例上（命中它
    /// 本地的 KV 缓存）。头**名字**由宿主决定（compat），值是我们这个会话。
    session_id: Option<String>,
    http: ureq::Agent,
    /// 非流式用的 agent：不设 `recv_body` 超时（见 `agent_slow`）。
    http_slow: ureq::Agent,
    retry: RetryPolicy,
    worker: std::sync::Arc<std::sync::Mutex<Option<crate::server::http_worker::SharedWorker>>>,
    worker_pid: std::sync::Arc<std::sync::atomic::AtomicU32>,
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

/// 一次 HTTP 失败：状态码 + **响应体**。
///
/// 为什么要带上正文：宿主的拒绝理由全在正文里，`400 Bad Request` 本身什么
/// 都没说。没有它，用户看到的错误无法自查；有了它，"schema 里的某个构造
/// 后端不认"这种事才有可能被自动识别并降级重试。
#[derive(Debug)]
pub struct HttpFailure {
    pub status: u16,
    pub body: String,
}

impl std::fmt::Display for HttpFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.body.trim().is_empty() {
            write!(f, "HTTP {}", self.status)
        } else {
            // 正文可能有几 KB 的栈；截断到能看清原因的长度。
            let body: String = self.body.chars().take(600).collect();
            write!(f, "HTTP {}: {}", self.status, body.trim())
        }
    }
}

impl std::error::Error for HttpFailure {}

/// 这个失败是不是"宿主不认 strict 模式的工具 schema"。
///
/// 有一类后端（把工具 schema 编译成语法的那些）在 schema 复杂到一定程度时
/// 直接拒收整个请求。错误信息里会出现 "compiled grammar" 与 "too large"。
/// 认出来之后**去掉 strict 再试一次**——不认的话用户只能自己去 `models.yml`
/// 里关掉 `toolStrictMode`，而错误信息不会提示这一点。
fn is_strict_rejection(e: &anyhow::Error) -> bool {
    let Some(f) = e.downcast_ref::<HttpFailure>() else {
        return false;
    };
    if f.status != 400 && f.status != 422 {
        return false;
    }
    let body = f.body.to_ascii_lowercase();
    // 判得**宽**：各家措辞不一样，而误判的代价只是"多发一次不带 strict 的
    // 请求"，漏判的代价是用户得自己去翻 `models.yml` 关掉 strict——而错误
    // 信息里不会提示这一点。
    //
    // 注意 `tool parameters schema` / `invalid schema for function` 这类说法
    // 里根本没有 "strict" 这个词：宿主说的是"schema 我不认"，而 schema 是
    // 我们为了 strict 才改写成那个形状的，所以它们同样指向这一档。
    (body.contains("compiled grammar") && body.contains("too large"))
        || (body.contains("strict") && (body.contains("tool") || body.contains("schema")))
        || body.contains("tool parameters")
        || body.contains("invalid schema for function")
        || body.contains("structured output")
}

/// Is this failure worth another try?
///
/// Yes for the transient classes (timeouts, socket errors, a dead connection,
/// DNS, 408/409/425/429, 5xx). No for anything that means "the request or the
/// answer is wrong" (bad URI, protocol error, TLS, redirect loop, 4xx other
/// than the ones above): retrying those just burns time and money.
fn retryable(e: &anyhow::Error) -> bool {
    if let Some(f) = e.downcast_ref::<HttpFailure>() {
        return matches!(f.status, 408 | 409 | 425 | 429 | 500 | 502 | 503 | 504);
    }
    if let Some(u) = e.downcast_ref::<ureq::Error>() {
        return match u {
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


impl Client {
    /// 这一轮是怎么结束的。
    ///
    /// 默认相信宿主报的 `finish_reason`。`supports_finish_reason: false` 的
    /// 宿主根本不报这个字段，那就只剩一条线索：这轮有没有要工具。少了这条
    /// 推断，带工具调用的一轮会被当成普通停止，循环层就不会继续跑。
    fn stop_reason(&self, raw: Option<String>, has_tool_calls: bool) -> StopReason {
        if self.compat.supports_finish_reason {
            return parse_finish_reason(raw);
        }
        if has_tool_calls {
            StopReason::ToolCalls
        } else {
            StopReason::Stop
        }
    }

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
    pub fn switch_model(
        &mut self,
        base_url: &str,
        api_key: &str,
        model: &str,
        user: Option<&crate::server::ai::compat::Compat>,
    ) {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self.api_key = api_key.to_string();
        self.model = model.to_string();
        self.compat = crate::server::ai::compat::resolve(&self.base_url, &self.model, user);
        self.user_compat = user.cloned();
    }
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        let worker = crate::server::http_worker::HttpWorker::spawn().ok();
        let pid = worker.as_ref().and_then(crate::server::http_worker::HttpWorker::pid).unwrap_or(0);
        let base_url = base_url.trim_end_matches('/').to_string();
        let compat = crate::server::ai::compat::resolve(&base_url, model, None);
        Self {
            base_url, api_key: api_key.to_string(), model: model.to_string(), compat,
            user_compat: None,
            session_id: None,
            http: Self::agent(), http_slow: Self::agent_slow(), retry: RetryPolicy::default(),
            worker: std::sync::Arc::new(std::sync::Mutex::new(worker)),
            worker_pid: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(pid)),
        }
    }

    /// Layer the model's `compat:` block from `models.yml` over detection.
    ///
    /// Separate from [`Client::new`] so the many call sites that only need a
    /// throwaway client (tests, one-shot helpers) stay one line. Re-resolves
    /// from scratch rather than merging, so a second call with different
    /// switches cannot accumulate.
    pub fn with_compat(mut self, user: Option<&crate::server::ai::compat::Compat>) -> Self {
        self.compat = crate::server::ai::compat::resolve(&self.base_url, &self.model, user);
        self.user_compat = user.cloned();
        self
    }

    /// The resolved switches this client will send with.
    pub fn compat(&self) -> &crate::server::ai::compat::Resolved {
        &self.compat
    }

    /// The user's `compat:` block, serialized for storage. `None` when the
    /// model entry had none.
    ///
    /// Serialization is infallible here (the struct is plain data), so a
    /// failure would mean a bug, not bad input — hence the `expect`. It
    /// serializes as a JSON object; `models.yml` uses the same camelCase keys,
    /// so a stored round reads back as the block the user typed.
    pub fn user_compat_json(&self) -> Option<String> {
        self.user_compat
            .as_ref()
            .map(|c| serde_json::to_string(c).expect("compat serialization is infallible"))
    }

    pub fn interrupt_worker(&self) {
        let pid = self.worker_pid.swap(0, std::sync::atomic::Ordering::AcqRel);
        if pid != 0 { crate::server::http_worker::HttpWorker::kill_pid(pid); }
        let fresh = crate::server::http_worker::HttpWorker::spawn();
        let Ok(fresh) = fresh else { return };
        let new_pid = crate::server::http_worker::HttpWorker::pid(&fresh).unwrap_or(0);
        if let Ok(mut slot) = self.worker.lock() {
            let old = slot.replace(fresh);
            self.worker_pid.store(new_pid, std::sync::atomic::Ordering::Release);
            if let Some(old) = old {
                std::thread::spawn(move || {
                    if let Ok(mut old) = old.lock() { old.reap(); }
                });
            }
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
    // 没有 body/global 总时限：一次回复可以合法地流好几分钟，总量 deadline
    // 会把话说到一半掐断。
    //
    // 但**接收体的 `SO_RCVTIMEO` 必须设**——不是为了掐断长回复，而是为了让
    // 打断能生效。`read` 是阻塞的，而打断只把原子 flag 设成 true；如果上游
    // 恰好停在两个 chunk 之间（模型在思考、网关在缓冲），tur n 线程就卡在
    // `read` 里，`on_delta` 永远不被调用，flag 永远是"没人看"——**按 Esc
    // 没反应就是这个**。
    //
    // `recv_body` 在 ureq 里落到 `TcpStream::set_read_timeout`，是**每次
    // read** 的超时（不是总量）。所以设成 1 秒：正常收流看不出区别（数据
    // 到了就返回），空闲那 1 秒返回一个超时错误，读循环据此查一次 flag 再
    // 继续读。
    //
    // 注意那个错误的形状——见 `is_body_read_timeout`：ureq 把它包成
    // `ErrorKind::Other`，不是 `TimedOut`。认错了就会把"停顿一秒"当成
    // "流断了"。
    // `recv_body` 超时在读循环里长什么样。
    //
    // **不是 `io::ErrorKind::TimedOut`**，这是个反直觉的地方：ureq 把
    // `Error::Timeout(RecvBody)` 包成 `io::Error` 时走的是
    // `Error::into_io()`，而非 `Io` 变体一律落到 `ErrorKind::Other`
    // （`ureq-3.4.2/src/error.rs:196`）。只认 `TimedOut` 的写法永远不会命中，
    // 结果是"上游停顿超过 1 秒 = 流断"——而 1 秒正是我们去查打断 flag 的间隔。
    //
    // 两处都要认：`TimedOut` 是真 io 层超时（worker 路径直接返回
    // `io::Error`），`ureq::Error::Timeout` 是 ureq 的包法。
    fn is_body_read_timeout(e: &std::io::Error) -> bool {
        if e.kind() == std::io::ErrorKind::TimedOut {
            return true;
        }
        matches!(
            e.get_ref().and_then(|s| s.downcast_ref::<ureq::Error>()),
            Some(ureq::Error::Timeout(_))
        )
    }

    // 非流式请求用的 agent。
    //
    // 唯一的区别是**不设 `recv_body` 超时**：那个 1 秒是为了让流式读有机会
    // 查打断 flag，而 `complete` 是一次性把整个响应体读完——套上 1 秒，任何
    // 慢于 1 秒的回复都会在读 body 时炸掉（而且这个错误不在 `post_retrying`
    // 的重试范围内，因为它发生在重试之后）。总时限由 `timeout_recv_response`
    // 兜底。
    fn agent_slow() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(300)))
            .http_status_as_error(false)
            .build()
            .into()
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(120)))
            .timeout_recv_body(Some(std::time::Duration::from_secs(1)))
            // 非 2xx 由我们自己处理：**要读正文**。ureq 默认把状态码变成
            // 一个不带正文的错误，而拒收的理由全在正文里。
            .http_status_as_error(false)
            .build()
            .into()
    }

    // Full chat/completions URL.
    /// 告诉这个 client 它属于哪个会话。只在宿主声明了
    /// `prompt_cache_session_header` 时才有意义——那个头让宿主把同一会话的
    /// 请求粘到同一个后端实例，命中它本地的 KV 缓存。
    pub fn set_session_id(&mut self, id: impl Into<String>) {
        self.session_id = Some(id.into());
    }

    /// 那些"不算在 body 里、但请求要带上"的头。
    fn extra_headers(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(name) = &self.compat.prompt_cache_session_header
            && let Some(sid) = &self.session_id
        {
            out.push((name.clone(), sid.clone()));
        }
        out
    }

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
    fn post(
        &self,
        agent: &ureq::Agent,
        body: &serde_json::Value,
    ) -> anyhow::Result<ureq::http::Response<ureq::Body>> {
        let mut req = agent
            .post(self.endpoint())
            .header("Authorization", &format!("Bearer {}", self.api_key));
        // 与 worker 路径共用一个来源，否则会出现"直连能粘会话、worker 不能"
        // 这种只在打断/降级时才暴露的差异。
        for (name, value) in self.extra_headers() {
            req = req.header(name, value);
        }
        let resp = req.send_json(body).context("HTTP request failed")?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(resp);
        }
        // 把正文读出来当错误信息。读不动（连接已经断了之类）也要保底给出
        // 状态码——用户至少要看到 4xx 还是 5xx。
        let body = resp
            .into_body()
            .read_to_string()
            .unwrap_or_else(|_| String::new());
        Err(anyhow::Error::new(HttpFailure { status, body }))
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
        agent: &ureq::Agent,
        body: &serde_json::Value,
    ) -> anyhow::Result<ureq::http::Response<ureq::Body>> {
        let mut n = 1;
        loop {
            match self.post(agent, body) {
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
        let messages = messages_to_wire(&ctx.messages, &self.compat);
        let tools = serde_json::to_value(&ctx.tools).expect("tool serialization is infallible");
        let body = body_json(
            &self.model,
            max_tokens,
            false,
            &messages,
            &tools,
            &self.compat,
            self.session_id.as_deref(),
        );
        let resp: ChatResponse = self
            .post_retrying(&self.http_slow, &body)?
            .body_mut()
            .read_json()
            .context("response is not valid JSON")?;

        let choice = resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("response has no choices"))?;

        let has_tools = !choice.message.tool_calls.is_empty();
        Ok(AssistantMessage {
            content: choice.message.content.unwrap_or_default(),
            tool_calls: choice.message.tool_calls,
            stop_reason: self.stop_reason(choice.finish_reason, has_tools),
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
        cancel: &std::sync::atomic::AtomicBool,
        mut on_delta: impl FnMut(&str) -> bool,
        // Returning false = interrupt: stop reading and disconnect. Same
        // contract as `on_delta`; a thinking-only stream (no content chunks)
        // would otherwise be uninterruptible until the first token.
        mut on_reasoning: impl FnMut(&str) -> bool,
    ) -> anyhow::Result<AssistantMessage> {
        let emitted = std::cell::Cell::new(false);
        let mut n = 1;
        // 宿主拒了 strict 的工具 schema 之后，去掉 strict 重试一次。
        let mut no_strict = false;
        loop {
            let r = self.stream_once(
                ctx,
                max_tokens,
                cancel,
                no_strict,
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
                // 这个降级**不睡**：宿主明确说了是 schema 的问题，再发一次
                // 不带 strict 的立刻就能过。睡一秒只是白等。
                Err(e) if !emitted.get() && !no_strict && is_strict_rejection(&e) => {
                    crate::server::log::warn(
                        &self.model,
                        format!("宿主拒了 strict 的工具 schema，去掉 strict 重试一次：{e:#}"),
                    );
                    no_strict = true;
                    n += 1;
                    continue;
                }
                Err(e) => {
                    // Already streaming text: no retry, whatever the cause.
                    //
                    // 打断同样不重试：那条错误是**我们**杀 worker 造成的
                    // （预期控制流，不是故障）。重试会先按退避睡 400ms、
                    // 再用替补 worker 发一次全新请求——用户按了 Esc，反而
                    // 多等一次完整往返，这就是"刚发出就打断"的体感延迟。
                    let again = !emitted.get()
                        && !cancel.load(std::sync::atomic::Ordering::Relaxed)
                        && self.should_retry(n, &e);
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
    #[allow(clippy::too_many_arguments)]
    fn stream_once(
        &self,
        ctx: &Context,
        max_tokens: u32,
        cancel: &std::sync::atomic::AtomicBool,
        no_strict: bool,
        mut on_delta: impl FnMut(&str) -> bool,
        mut on_reasoning: impl FnMut(&str) -> bool,
    ) -> anyhow::Result<AssistantMessage> {
        // `no_strict` 是"这次别发 strict"的降级开关：宿主拒了 strict 的 schema
        // 之后重发一次。用一个改了开关的副本，而不是给 `body_json` 再加参数
        // ——那样"发什么"仍然只有一处决定。
        let mut compat = self.compat.clone();
        if no_strict {
            compat.supports_strict_mode = false;
        }
        let compat = &compat;
        let messages = messages_to_wire(&ctx.messages, compat);
        let tools = serde_json::to_value(&ctx.tools).expect("Message serialization is infallible");
        let body = body_json(
            &self.model,
            max_tokens,
            true,
            &messages,
            &tools,
            compat,
            self.session_id.as_deref(),
        );
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let worker_slot = self.worker.lock().ok().and_then(|slot| slot.clone());
        let worker_used = worker_slot.is_some();
        if let Some(worker_slot) = worker_slot {
            let base = self.base_url.clone();
            let key = self.api_key.clone();
            let request_body = body.clone();
            let headers = self.extra_headers();
            std::thread::spawn(move || {
                let result = worker_slot.lock().map_err(|_| anyhow!("HTTP worker 锁损坏")).and_then(|mut w| {
                    w.stream(&base, &key, request_body, &headers, |line| tx.send(line.to_string()).is_ok())
                });
                if let Err(e) = result { let _ = tx.send(format!("__WORKER_ERROR__{e}")); }
            });
        }
        let reader: Box<dyn std::io::BufRead> = if worker_used {
            Box::new(std::io::BufReader::new(ChannelReader { rx, pending: Vec::new() }))
        } else {
            Box::new(std::io::BufReader::new(
                self.post(&self.http, &body)?.into_body().into_reader(),
            ))
        };

        let mut content = String::new();
        let mut reasoning = String::new();
        let mut finish_reason: Option<String> = None;
        // Leaked-marker filters. `enabled` is per host, so the default path
        // (a clean gateway) is a direct `push_str` with no scanning.
        // 推理循环守护：只有宿主声明了才建，省掉每条 delta 上的扫描。
        let mut loop_guard = crate::server::ai::loop_guard::ThinkingLoopGuard::for_compat(compat);
        // 带内解析：宿主把工具调用/思考塞在正文里时，只有它能把它们捞出来。
        // 没有这个，剥掉标记之后工具**不会执行**——模型看到自己调了工具却拿
        // 不到结果，于是重复输出或者开始编，钱和时间都白花。
        let mut scanner = crate::server::ai::inband::scanner_for(compat);
        // 从正文里解析出来的调用。id 是这里生成的（确定性：按顺序编号），
        // 因为宿主根本不给 id。
        let mut healed_calls: Vec<crate::server::ai::types::ToolCall> = Vec::new();
        let mut content_filter = TemplateMarkupFilter::for_compat(compat);
        let mut reasoning_filter = TemplateMarkupFilter::for_compat(compat);
        let mut usage = Usage::default();
        let mut interrupted = false;
        let mut tool_calls = ToolCallAccum::default();

        // 读行必须**容得下空闲**：`recv_body` 是每次 read 的 1 秒超时，上游
        // 两个 chunk 之间隔久了就是 `TimedOut`。那不是故障——是让本轮有机会
        // 查一次打断 flag。真故障（连接断、协议错）照旧上抛。
        //
        // 用 `read_until` 而不是 `lines()`：后者把 `TimedOut` 直接当成
        // `Err` 结束整个流，空闲一秒就等于"流断了"。
        let mut reader = reader;
        let mut pending: Vec<u8> = Vec::new();
        // 首事件超时：本地冷启动的模型可能几十秒才吐第一个 token，所以默认
        // 不限；但有的宿主会在提示词处理上永久卡住，那时"一直等"比报错更糟。
        // 由 `stream_first_event_timeout_ms` 按宿主打开，`0` = 不限。
        let started = std::time::Instant::now();
        let first_event_budget = compat
            .stream_first_event_timeout_ms
            .filter(|ms| *ms > 0)
            .map(std::time::Duration::from_millis);
        let mut got_first = false;
        // 事件间空闲上限：首事件之后，任意两次 `data:` 之间超过它就判卡死。
        // 与首事件超时互补——首事件管"从来就没开始"，这个管"开始了但是停
        // 住了"。`0` = 不限。
        let idle_budget = compat
            .stream_idle_timeout_ms
            .filter(|ms| *ms > 0)
            .map(std::time::Duration::from_millis);
        let mut last_event = std::time::Instant::now();
        loop {
            pending.clear();
            match std::io::BufRead::read_until(&mut reader, b'\n', &mut pending) {
                Ok(0) => break, // 上游正常收流（EOF）
                Ok(_) => {}
                Err(e) if Self::is_body_read_timeout(&e) => {
                    // 空闲。给打断一次机会：flag 已设 = 用户按了 Esc，
                    // 立刻收工（当成本轮被中断，不是错误）。
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        interrupted = true;
                        break;
                    }
                    // 还没见到第一个事件，又过了宿主自己声明的上限：这不是
                    // "慢"，是"卡住了"。
                    if !got_first
                        && let Some(budget) = first_event_budget
                        && started.elapsed() > budget
                    {
                        anyhow::bail!(
                            "上游 {:?} 内没有吐出第一个事件，判定为卡住（{} 的首事件超时；\
                             本地冷启动慢的宿主不该设这个值）",
                            budget,
                            self.model,
                        );
                    }
                    // 已经开始收流了，那就不是"慢"，是"停住了"。
                    if got_first
                        && let Some(budget) = idle_budget
                        && last_event.elapsed() > budget
                    {
                        anyhow::bail!(
                            "上游 {:?} 内没有新事件，判定为卡死（{} 的事件间空闲超时）",
                            budget,
                            self.model,
                        );
                    }
                    continue;
                }
                Err(e) => {
                    // 打断：worker 被主动杀掉后，管道 EOF / closed 是**预期
                    // 控制流**。当成中断收工，不产生错误文本——错误文本会
                    // 进转录，用户看到的 `stream read interrupted: worker
                    // closed` 就是这么来的。
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

            // Reasoning streams through to upstream (UI displays live)
            // and accumulates into the final message. Three field names
            // are accepted (pi's list): reasoning_content / reasoning /
            // reasoning_text
            for field in ["reasoning_content", "reasoning", "reasoning_text"] {
                if let Some(r) = delta.get(field).and_then(|v| v.as_str()) {
                    // Some hosts resend the **whole** reasoning text every chunk
                    // instead of a delta. Appending blindly doubles the text.
                    // Detect it: the new chunk starts with everything we already
                    // have, so the increment is its tail.
                    let fresh = if self.compat.reasoning_deltas_may_be_cumulative
                        && !reasoning.is_empty()
                        && r.len() > reasoning.len()
                        && r.starts_with(reasoning.as_str())
                    {
                        r[reasoning.len()..].to_string()
                    } else if self.compat.reasoning_deltas_may_be_cumulative
                        && r == reasoning.as_str()
                    {
                        String::new() // a pure repeat: nothing new
                    } else {
                        r.to_string()
                    };
                    let mut emitted = String::new();
                    reasoning_filter.feed(&fresh, &mut emitted);
                    // 循环守护看到的是**过滤后**的文本：过滤掉标记的正文
                    // 才是模型真正产出的内容。
                    if let Some(reason) = loop_guard.as_mut().and_then(|g| g.push(&emitted)) {
                        // 明确报错而不是静默截断：本轮丢了，调用方会按可重试
                        // 失败处理，用户可以重发。
                        anyhow::bail!(
                            "推理陷入循环，本轮丢弃（{reason}；{}）",
                            self.model
                        );
                    }
                    reasoning.push_str(&emitted);
                    if !emitted.is_empty() && !on_reasoning(&emitted) {
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
                if emitted.is_empty() {
                    continue;
                }
                // 上游同时给了结构化的 `tool_calls` 时，正文里的那份是**副本**
                // ——把它当正文显示出去就行，不能再合成一个调用（否则同一次
                // 调用会执行两遍）。
                let structured = delta
                    .get("tool_calls")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| !a.is_empty());
                match scanner.as_mut() {
                    Some(sc) => {
                        for ev in sc.feed(&emitted) {
                            match ev {
                                crate::server::ai::inband::ScanEvent::Text(t) => {
                                    content.push_str(&t);
                                    if !on_delta(&t) {
                                        interrupted = true;
                                    }
                                }
                                crate::server::ai::inband::ScanEvent::Thinking(t) => {
                                    reasoning.push_str(&t);
                                    if !on_reasoning(&t) {
                                        interrupted = true;
                                    }
                                }
                                crate::server::ai::inband::ScanEvent::ToolCall {
                                    name,
                                    arguments,
                                } => {
                                    if !structured {
                                        healed_calls.push(crate::server::ai::types::ToolCall {
                                            id: format!("call_healed_{}", healed_calls.len()),
                                            kind: "function".into(),
                                            function: crate::server::ai::types::FunctionCall {
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
                        if !on_delta(&emitted) {
                            interrupted = true;
                        }
                    }
                }
                if interrupted {
                    break;
                }
            }
            // Tool-call fragments (see the ToolCallAccum docs)
            tool_calls.feed(delta);
            if let Some(f) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                finish_reason = Some(f.to_string());
            }
        }

        // 流结束：scanner 扣住的尾巴要交出来（可能是没闭合的调用、也可能只是
        // 普通正文）。**必须做**——不做就丢文本。
        if !interrupted
            && let Some(sc) = scanner.as_mut()
        {
            for ev in sc.flush() {
                match ev {
                    crate::server::ai::inband::ScanEvent::Text(t) => {
                        content.push_str(&t);
                        on_delta(&t);
                    }
                    crate::server::ai::inband::ScanEvent::Thinking(t) => {
                        reasoning.push_str(&t);
                        on_reasoning(&t);
                    }
                    crate::server::ai::inband::ScanEvent::ToolCall { name, arguments } => {
                        healed_calls.push(crate::server::ai::types::ToolCall {
                            id: format!("call_healed_{}", healed_calls.len()),
                            kind: "function".into(),
                            function: crate::server::ai::types::FunctionCall { name, arguments },
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
                on_delta(&tail);
            }
            let mut rtail = String::new();
            reasoning_filter.flush(&mut rtail);
            if !rtail.is_empty() {
                reasoning.push_str(&rtail);
                on_reasoning(&rtail);
            }
        }

        // Some gateways signal a full context window with a perfectly normal
        // response that carries `finish_reason: "length"` and **no text at
        // all**. Left alone that surfaces as "the model said nothing", which
        // sends the user hunting for a model problem. Name the real cause.
        if !interrupted
            && compat.empty_length_finish_is_context_error
            && content.is_empty()
            && reasoning.is_empty()
            && finish_reason.as_deref() == Some("length")
        {
            anyhow::bail!(
                "模型以 length 收尾且没有输出任何内容：上下文窗口多半已经满了（这个宿主用空响应表示它）"
            );
        }

        let mut tool_calls = tool_calls.finish();
        let healed = healed_calls.is_empty() == false;
        tool_calls.extend(healed_calls);
        let stop_reason = if interrupted {
            StopReason::Interrupted
        } else if healed {
            // 从正文里解析出调用时，宿主的 `finish_reason` 往往是 `stop`
            // ——它根本不知道模型调了工具。不提升的话循环层就"这轮结束了"，
            // 工具永远不执行。
            StopReason::ToolCalls
        } else {
            self.stop_reason(finish_reason, !tool_calls.is_empty())
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let http = |status: u16| {
            anyhow::Error::new(HttpFailure {
                status,
                body: String::new(),
            })
        };
        for code in [408, 409, 425, 429, 500, 502, 503, 504] {
            assert!(retryable(&http(code)), "{code} 应当重试");
        }
        for code in [400, 401, 403, 404, 422] {
            assert!(!retryable(&http(code)), "{code} 重试没意义");
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


    /// 宿主"不认 strict 的工具 schema"要被认出来——认出来才能降级重试。
    #[test]
    fn a_strict_schema_rejection_is_recognized() {
        let fail = |status: u16, body: &str| {
            anyhow::Error::new(HttpFailure {
                status,
                body: body.to_string(),
            })
        };
        // 把工具 schema 编译成 grammar 的后端的原话。
        assert!(is_strict_rejection(&fail(
            400,
            r#"{"error":{"message":"invalid_request_error: compiled grammar is too large"}}"#
        )));
        // 另一家的措辞。
        assert!(is_strict_rejection(&fail(
            400,
            r#"{"error":{"message":"tool parameters schema is not supported"}}"#
        )));
        // 不是 400/422 就不降级：那个状态的成因多半跟 schema 无关。
        assert!(!is_strict_rejection(&fail(404, "tool schema")));
        // 底下没有响应体时不能瞎猜。
        assert!(!is_strict_rejection(&fail(400, "")));
        // 跟工具和 strict 都无关的 400 不该触发降级。
        assert!(!is_strict_rejection(&fail(
            400,
            r#"{"error":{"message":"unknown model gpt-9"}}"#
        )));
    }

    /// 错误信息里要带上宿主给的正文——`400 Bad Request` 本身什么都没说。
    #[test]
    fn the_host_body_reaches_the_error_message() {
        let e = anyhow::Error::new(HttpFailure {
            status: 400,
            body: r#"{"error":{"message":"temperature is not supported"}}"#.to_string(),
        });
        let text = format!("{e:#}");
        assert!(text.contains("400"), "{text}");
        assert!(text.contains("temperature is not supported"), "{text}");
    }

    /// 会话粘性头只在宿主声明了名字、且我们真有会话 id 时才发——多发一个头
    /// 就是一个被拒的机会。
    #[test]
    fn the_session_affinity_header_is_only_sent_when_the_host_wants_it() {
        let mut c = client();
        assert!(c.extra_headers().is_empty(), "默认一个头都不多发");

        c.compat.prompt_cache_session_header = Some("x-session-id".into());
        assert!(c.extra_headers().is_empty(), "没有会话 id 时也不发");

        c.set_session_id("s-42");
        assert_eq!(
            c.extra_headers(),
            vec![("x-session-id".to_string(), "s-42".to_string())]
        );
    }

    /// 宿主不报 `finish_reason` 时只剩一条线索：这轮要不要工具。少了这条
    /// 推断，带工具调用的一轮会被当成普通停止，循环层就不往下走了。
    #[test]
    fn a_host_without_finish_reason_falls_back_to_the_tool_signal() {
        let mut c = client();
        assert_eq!(
            c.stop_reason(Some("length".into()), false),
            StopReason::Length,
            "默认相信宿主报的值"
        );

        c.compat.supports_finish_reason = false;
        assert_eq!(c.stop_reason(None, true), StopReason::ToolCalls);
        assert_eq!(c.stop_reason(None, false), StopReason::Stop);
        assert_eq!(
            c.stop_reason(Some("length".into()), true),
            StopReason::ToolCalls,
            "宿主说了不报，那它报了也不看"
        );
    }

    /// 真机探针：本机网关必须接受改造后发出的请求体。
    #[test]
    #[ignore = "真机：需要 127.0.0.1:8788 上的网关"]
    fn the_live_gateway_accepts_what_we_send() {
        use crate::server::ai::types::{Context, Message};

        let mut c = Client::new("http://127.0.0.1:8788/v1", "sk-anything", "gpt-5.6-luna");
        c.set_session_id("live-probe-1");

        let mut ctx = Context::new();
        ctx.messages.push(Message::User {
            content: "只回两个字：收到".into(),
        });

        // 1. 保守基线：网关照常回答。
        let msg = c.complete(&ctx, 64).expect("默认路径必须能被网关接受");
        assert_eq!(msg.content.trim(), "收到", "网关的回复：{:?}", msg.content);

        // 2. 打开缓存键（治命中率那一项）：网关必须接受，不 400。
        let mut compat = c.compat().clone();
        compat.supports_prompt_cache_key = true;
        let body = crate::server::ai::assemble::body_json(
            "gpt-5.6-luna", 64, false, &serde_json::json!([{"role":"user","content":"只回两个字：收到"}]),
            &serde_json::json!([]), &compat, Some("live-probe-1"),
        );
        assert_eq!(body["prompt_cache_key"], "live-probe-1");
        let plain_agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_recv_response(Some(std::time::Duration::from_secs(60)))
            .build()
            .into();
        let resp = plain_agent
            .post("http://127.0.0.1:8788/v1/chat/completions")
            .header("Authorization", "Bearer sk-anything")
            .send_json(&body);
        match resp {
            Ok(r) => {
                let v: serde_json::Value = r.into_body().read_json().expect("网关返回 JSON");
                assert!(
                    v["choices"][0]["message"]["content"].as_str().is_some_and(|s| !s.is_empty()),
                    "网关回答了：{v}"
                );
            }
            Err(e) => panic!("网关拒了带 prompt_cache_key 的请求：{e}"),
        }
    }
}
