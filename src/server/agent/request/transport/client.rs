//! AI client — mirrors pi's packages/ai/src/api/openai-completions.ts: turn a `Context` into an HTTP
//! request, and the response (or SSE stream) back into an `AssistantMessage`.

use anyhow::Context as _;
pub(crate) struct ChannelReader { pub(crate) rx: std::sync::mpsc::Receiver<String>, pub(crate) pending: Vec<u8> }
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

use crate::server::agent::request::message::StopReason;
use crate::server::agent::request::inbound::response::parse_finish_reason;

// Connection details for one provider.
#[derive(Debug, Clone)]
pub struct Client {
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) model: String,
    /// Resolved compat switches for this endpoint + model, recomputed when either changes; read by
    /// `body_json`. Never read from config at request time.
    pub(crate) compat: crate::server::agent::request::assembly::compat::Resolved,
    /// The `compat:` block as the user wrote it: the half that is *not* recomputable.
    pub(crate) user_compat: Option<crate::server::agent::request::assembly::compat::Compat>,
    /// The session id for `prompt_cache_session_header`; `None` = that header is not sent.
    pub(crate) session_id: Option<String>,
    pub(crate) http: ureq::Agent,
    pub(crate) retry: RetryPolicy,
    /// This client's worker slot: `None` = requests go direct. The `Arc` is shared by a session's clones.
    pub(crate) worker: std::sync::Arc<std::sync::Mutex<Option<crate::server::agent::request::transport::worker::SharedWorker>>>,
    /// The worker's pid, `0` = none; an interrupt kills it. One worker per session.
    pub(crate) worker_pid: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

/// The one protocol this client speaks. Stored with every round (see
/// `store::RoundMeta`) so replay is a data question, not a guess.
pub const PROTOCOL: &str = "openai-chat-completions";

/// Retry policy: three attempts total, 400 ms then 800 ms with ±25% jitter. Streaming requests are
/// retried **only while nothing has been emitted**.
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
    pub(crate) fn delay_ms(&self, attempt: u32) -> u64 {
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

/// One HTTP failure: the status code plus the **body** — hosts put their reason there.
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
            // The body can be kilobytes of stack; truncate to what shows the reason.
            let body: String = self.body.chars().take(600).collect();
            write!(f, "HTTP {}: {}", self.status, body.trim())
        }
    }
}

impl std::error::Error for HttpFailure {}

/// Is this the host refusing a strict tool schema? Then retry once without strict.
pub(crate) fn is_strict_rejection(e: &anyhow::Error) -> bool {
    let Some(f) = e.downcast_ref::<HttpFailure>() else {
        return false;
    };
    if f.status != 400 && f.status != 422 {
        return false;
    }
    let body = f.body.to_ascii_lowercase();
    // Judged **loosely**: a false positive costs one extra request; a miss costs the user a manual edit.
    (body.contains("compiled grammar") && body.contains("too large"))
        || (body.contains("strict") && (body.contains("tool") || body.contains("schema")))
        || body.contains("tool parameters")
        || body.contains("invalid schema for function")
        || body.contains("structured output")
}

/// Is this failure worth another try? Yes for timeouts, socket errors, 408/409/425/429 and 5xx; no
/// for anything that means the request or answer is wrong.
pub(crate) fn retryable(e: &anyhow::Error) -> bool {
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
    /// How the round ended: the host's `finish_reason` when it reports one, else "did it ask for tools".
    pub(crate) fn stop_reason(&self, raw: Option<String>, has_tool_calls: bool) -> StopReason {
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
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    // Wire protocol this client speaks. Recorded per round: a future second
    // protocol must store its own id, so replay never assumes a shape.
    pub fn protocol(&self) -> &'static str {
        PROTOCOL
    }

    // Switch model within the session (endpoint and key switch along). Not persisted.
    pub fn switch_model(
        &mut self,
        base_url: &str,
        api_key: &str,
        model: &str,
        user: Option<&crate::server::agent::request::assembly::compat::Compat>,
    ) {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self.api_key = api_key.to_string();
        self.model = model.to_string();
        self.compat = crate::server::agent::request::assembly::compat::resolve(&self.base_url, &self.model, user);
        self.user_compat = user.cloned();
    }
    /// An unclaimed client: connection details, no worker process (`new` builds templates).
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        let compat = crate::server::agent::request::assembly::compat::resolve(&base_url, model, None);
        Self {
            base_url, api_key: api_key.to_string(), model: model.to_string(), compat,
            user_compat: None,
            session_id: None,
            http: Self::agent(), retry: RetryPolicy::default(),
            worker: std::sync::Arc::new(std::sync::Mutex::new(None)),
            worker_pid: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
        }
    }

    /// Claim a worker of this session's own. An interrupt needs it: killing the child is what unblocks
    /// the read. One worker per session, or two sessions kill each other's.
    pub fn with_own_worker(mut self) -> Self {
        let fresh = crate::server::agent::request::transport::worker::HttpWorker::spawn().ok();
        let pid = fresh.as_ref().and_then(crate::server::agent::request::transport::worker::HttpWorker::pid).unwrap_or(0);
        self.worker = std::sync::Arc::new(std::sync::Mutex::new(fresh));
        self.worker_pid = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(pid));
        self
    }

    /// Layer the model's `compat:` block over detection. Re-resolves from scratch, so a second call
    /// cannot accumulate.
    pub fn with_compat(mut self, user: Option<&crate::server::agent::request::assembly::compat::Compat>) -> Self {
        self.compat = crate::server::agent::request::assembly::compat::resolve(&self.base_url, &self.model, user);
        self.user_compat = user.cloned();
        self
    }

    /// The resolved switches this client will send with.
    pub fn compat(&self) -> &crate::server::agent::request::assembly::compat::Resolved {
        &self.compat
    }

    /// The user's `compat:` block, serialized for storage (plain data, so a failure is a bug).
    pub fn user_compat_json(&self) -> Option<String> {
        self.user_compat
            .as_ref()
            .map(|c| serde_json::to_string(c).expect("compat serialization is infallible"))
    }

    pub fn interrupt_worker(&self) {
        let pid = self.worker_pid.swap(0, std::sync::atomic::Ordering::AcqRel);
        if pid != 0 { crate::server::agent::request::transport::worker::HttpWorker::kill_pid(pid); }
        let fresh = crate::server::agent::request::transport::worker::HttpWorker::spawn();
        if let Ok(mut slot) = self.worker.lock() {
            match fresh {
                Ok(fresh) => {
                    let new_pid = crate::server::agent::request::transport::worker::HttpWorker::pid(&fresh).unwrap_or(0);
                    let old = slot.replace(fresh);
                    self.worker_pid.store(new_pid, std::sync::atomic::Ordering::Release);
                    if let Some(old) = old {
                        std::thread::spawn(move || {
                            if let Ok(mut old) = old.lock() { old.reap(); }
                        });
                    }
                }
                // The replacement would not start: clear the slot and go direct, or every round hits a dead pipe.
                Err(_) => {
                    slot.take();
                    self.worker_pid.store(0, std::sync::atomic::Ordering::Release);
                }
            }
        }
    }

    /// Replace the retry policy (tests use a tiny backoff; a future config key
    /// would use this too).
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    // Timeouts only where they cannot hurt a long reply: connect 30 s, response headers 120 s. The
    // receive body must have 1 s (see `is_body_read_timeout`), or an interrupt is never noticed.
    pub(crate) fn is_body_read_timeout(e: &std::io::Error) -> bool {
        if e.kind() == std::io::ErrorKind::TimedOut {
            return true;
        }
        matches!(
            e.get_ref().and_then(|s| s.downcast_ref::<ureq::Error>()),
            Some(ureq::Error::Timeout(_))
        )
    }

    pub(crate) fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(120)))
            .timeout_recv_body(Some(std::time::Duration::from_secs(1)))
            // Non-2xx is ours to handle: the body has to be read. ureq would otherwise turn it into an error
            // without one, and the reason is always in the body.
            .http_status_as_error(false)
            .build()
            .into()
    }

    /// Tell this client which session it belongs to; only meaningful when the host declares the header.
    pub fn set_session_id(&mut self, id: impl Into<String>) {
        self.session_id = Some(id.into());
    }

    /// Headers the request carries but the body does not.
    pub(crate) fn extra_headers(&self) -> Vec<(String, String)> {
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
    pub(crate) fn post(
        &self,
        agent: &ureq::Agent,
        body: &serde_json::Value,
    ) -> anyhow::Result<ureq::http::Response<ureq::Body>> {
        let mut req = agent
            .post(self.endpoint())
            .header("Authorization", &format!("Bearer {}", self.api_key));
        // One source with the worker path, or "direct sticks to the session, worker does not" appears only
        // under interruption.
        for (name, value) in self.extra_headers() {
            req = req.header(name, value);
        }
        let resp = req.send_json(body).context("HTTP request failed")?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(resp);
        }
        // Read the body as the error message; if that fails, the status code at least must survive.
        let body = resp
            .into_body()
            .read_to_string()
            .unwrap_or_else(|_| String::new());
        Err(anyhow::Error::new(HttpFailure { status, body }))
    }

    /// Log one failed try and say whether another is allowed.
    pub(crate) fn should_retry(&self, n: u32, e: &anyhow::Error) -> bool {
        if n < self.retry.attempts && retryable(e) {
            crate::server::platform::log::warn(
                &self.model,
                format!(
                    "request {n}/{} failed: {e:#}; retrying in {}ms",
                    self.retry.attempts,
                    self.retry.delay_ms(n)
                ),
            );
            true
        } else {
            crate::server::platform::log::error(
                &self.model,
                format!(
                    "{e:#}（{}）",
                    if n >= self.retry.attempts {
                        format!("gave up after {} attempts", self.retry.attempts)
                    } else {
                        "not a retryable error".to_string()
                    }
                ),
            );
            false
        }
    }

}

