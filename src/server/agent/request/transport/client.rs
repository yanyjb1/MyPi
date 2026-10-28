use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::server::agent::request::content::{BlockSink, RetrySink};
use crate::server::agent::request::message::AssistantMessage;
use crate::server::agent::request::protocols::openai::completion::compat::Compat;
use crate::server::agent::request::protocols::{
    Protocol, ProtocolId, ProtocolRequest, WireRequest,
};
use crate::server::agent::request::retry::{Failure, RetryClass, RetryPolicy, classify, notice};
use crate::server::agent::request::transport::worker::{HttpWorker, SharedWorker};

pub(crate) struct ChannelReader {
    pub(crate) rx: std::sync::mpsc::Receiver<String>,
    pub(crate) pending: Vec<u8>,
}

impl std::io::Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pending.is_empty() {
            let line = self.rx.recv().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "worker closed")
            })?;
            self.pending.extend_from_slice(line.as_bytes());
            self.pending.push(b'\n');
        }
        let n = out.len().min(self.pending.len());
        out[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}

pub struct Client {
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) model: String,
    pub(crate) session_id: Option<String>,
    protocol_id: ProtocolId,
    protocol: Box<dyn Protocol>,
    pub(crate) http: ureq::Agent,
    pub(crate) retry: RetryPolicy,
    pub(crate) worker: Arc<Mutex<Option<SharedWorker>>>,
    pub(crate) worker_pid: Arc<AtomicU32>,
}

impl Client {
    pub fn new(
        base_url: &str,
        api_key: &str,
        model: &str,
        protocol: ProtocolId,
        user: Option<&Compat>,
    ) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        let retry = RetryPolicy::default();
        Self {
            base_url: base_url.clone(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            session_id: None,
            protocol_id: protocol,
            protocol: protocol.build(&base_url, model, user),
            http: agent(retry.timeout),
            retry,
            worker: Arc::new(Mutex::new(None)),
            worker_pid: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn protocol(&self) -> &ProtocolId {
        &self.protocol_id
    }

    pub fn compat_json(&self) -> Option<String> {
        self.protocol.compat_json()
    }

    pub fn set_session_id(&mut self, id: impl Into<String>) {
        self.session_id = Some(id.into());
    }

    pub fn with_own_worker(mut self) -> Self {
        let fresh = HttpWorker::spawn().ok();
        let pid = fresh.as_ref().and_then(HttpWorker::pid).unwrap_or(0);
        self.worker = Arc::new(Mutex::new(fresh));
        self.worker_pid = Arc::new(AtomicU32::new(pid));
        self
    }

    pub fn with_compat(mut self, user: Option<&Compat>) -> Self {
        self.protocol = self.protocol_id.build(&self.base_url, &self.model, user);
        self
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self.http = agent(retry.timeout);
        self
    }

    pub fn switch_model(
        &mut self,
        base_url: &str,
        api_key: &str,
        model: &str,
        protocol: ProtocolId,
        user: Option<&Compat>,
    ) {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self.api_key = api_key.to_string();
        self.model = model.to_string();
        self.protocol_id = protocol;
        self.protocol = protocol.build(&self.base_url, model, user);
    }

    pub fn interrupt_worker(&self) {
        let pid = self.worker_pid.swap(0, Ordering::AcqRel);
        if pid != 0 {
            HttpWorker::kill_pid(pid);
        }
        let fresh = HttpWorker::spawn();
        if let Ok(mut slot) = self.worker.lock() {
            match fresh {
                Ok(fresh) => {
                    let new_pid = HttpWorker::pid(&fresh).unwrap_or(0);
                    let old = slot.replace(fresh);
                    self.worker_pid.store(new_pid, Ordering::Release);
                    if let Some(old) = old {
                        std::thread::spawn(move || {
                            if let Ok(mut old) = old.lock() {
                                old.reap();
                            }
                        });
                    }
                }
                Err(_) => {
                    slot.take();
                    self.worker_pid.store(0, Ordering::Release);
                }
            }
        }
    }

    pub fn complete(
        &self,
        req: &ProtocolRequest,
        sink: &mut dyn BlockSink,
        retry: &mut dyn RetrySink,
        cancel: &AtomicBool,
    ) -> anyhow::Result<AssistantMessage> {
        let mut emitted = false;
        let mut last: Option<anyhow::Error> = None;
        for attempt in 0..=self.retry.max_retries {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow::Error::new(Failure::Cancelled));
            }
            let wire = self.protocol.request(self, req)?;
            let result =
                crate::server::agent::request::transport::send(self, &wire).and_then(|reader| {
                    let mut tracked = Tracked {
                        inner: sink,
                        emitted: &mut emitted,
                    };
                    self.protocol.decode(reader, &mut tracked, cancel)
                });
            match result {
                Ok(message) => return Ok(message),
                Err(e) => {
                    let Some(failure) = e.downcast_ref::<Failure>() else {
                        return Err(e);
                    };
                    let class = classify(failure, emitted, cancel.load(Ordering::Relaxed));
                    if class == RetryClass::Fatal || attempt >= self.retry.max_retries {
                        crate::server::platform::log::error(&self.model, format!("{e:#}"));
                        return Err(e);
                    }
                    let n = attempt + 1;
                    retry.retry(&notice(
                        failure,
                        n,
                        self.retry.max_retries,
                        self.retry.timeout,
                    ));
                    crate::server::platform::log::warn(
                        &self.model,
                        format!(
                            "request failed: {e:#}; retrying {n}/{}",
                            self.retry.max_retries
                        ),
                    );
                    self.interrupt_worker();
                    std::thread::sleep(self.retry.interval);
                    last = Some(e);
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow::anyhow!("request failed")))
    }
}

struct Tracked<'a> {
    inner: &'a mut dyn BlockSink,
    emitted: &'a mut bool,
}

impl BlockSink for Tracked<'_> {
    fn block(&mut self, block: crate::server::storage::NewBlock, done: bool) -> bool {
        *self.emitted = true;
        self.inner.block(block, done)
    }
}

fn agent(timeout: std::time::Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(30)))
        .timeout_recv_response(Some(timeout))
        .timeout_recv_body(Some(std::time::Duration::from_secs(1)))
        .http_status_as_error(false)
        .build()
        .into()
}

pub(crate) fn send_error(e: ureq::Error) -> anyhow::Error {
    anyhow::Error::new(Failure::NoResponse {
        source: e.to_string(),
    })
}
