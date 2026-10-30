//! One provider connection: retry, cancellation, and the worker that owns the socket.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;

use crate::server::agent::request::live::Line;
use crate::server::agent::request::message::AssistantMessage;
use crate::server::agent::request::protocol::{Endpoint, Output, Protocol, ProtocolRequest};
use crate::server::agent::request::retry::{self, Class, Failure, RetryPolicy};
use crate::server::agent::request::transport::Provider;
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

/// One provider, one model, one protocol instance.
pub struct Client {
    endpoint: Endpoint,
    protocol: Box<dyn Protocol>,
    model: String,
    http: ureq::Agent,
    pub(crate) retry: RetryPolicy,
    worker: Mutex<Option<SharedWorker>>,
    pid: Arc<AtomicU32>,
}

impl Client {
    pub fn new(provider: &Provider, model: &str, retry: RetryPolicy) -> Self {
        let base_url = provider.base_url.trim_end_matches('/').to_string();
        Self {
            endpoint: Endpoint {
                base_url: base_url.clone(),
                api_key: provider.api_key.clone(),
                auth_header: provider.auth_header,
            },
            protocol: provider.protocol.build(&base_url, model),
            model: model.to_string(),
            http: agent(retry.timeout),
            retry,
            worker: Mutex::new(None),
            pid: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub(crate) fn http(&self) -> &ureq::Agent {
        &self.http
    }

    /// The worker, spawned on first use. `None` means every call goes direct.
    pub(crate) fn worker(&self) -> Option<SharedWorker> {
        let mut slot = self.worker.lock().ok()?;
        if slot.is_none() {
            let fresh = HttpWorker::spawn().ok()?;
            self.pid
                .store(HttpWorker::pid(&fresh).unwrap_or(0), Ordering::Release);
            *slot = Some(fresh);
        }
        slot.clone()
    }

    /// Kill the worker (and with it the socket) and put a fresh one in its place.
    fn replace_worker(&self) {
        let pid = self.pid.swap(0, Ordering::AcqRel);
        if pid != 0 {
            HttpWorker::kill_pid(pid);
        }
        let fresh = HttpWorker::spawn().ok();
        if let Ok(mut slot) = self.worker.lock() {
            match fresh {
                Some(fresh) => {
                    let new_pid = HttpWorker::pid(&fresh).unwrap_or(0);
                    let old = slot.replace(fresh);
                    self.pid.store(new_pid, Ordering::Release);
                    if let Some(old) = old {
                        std::thread::spawn(move || {
                            if let Ok(mut old) = old.lock() {
                                old.reap();
                            }
                        });
                    }
                }
                None => {
                    let _ = slot.take();
                    self.pid.store(0, Ordering::Release);
                }
            }
        }
    }

    /// One call, retried per policy. Every failure is said out loud exactly once.
    pub fn complete(
        &self,
        req: &ProtocolRequest,
        out: &mut dyn Output,
        line: &Line,
        cancel: &Arc<AtomicBool>,
    ) -> Result<AssistantMessage> {
        let done = Arc::new(AtomicBool::new(false));
        let watcher = {
            let cancel = cancel.clone();
            let done = done.clone();
            let pid = self.pid.clone();
            std::thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    if cancel.load(Ordering::Relaxed) {
                        let pid = pid.load(Ordering::Relaxed);
                        if pid != 0 {
                            HttpWorker::kill_pid(pid);
                        }
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            })
        };

        let mut emitted = false;
        let mut attempt = 0u32;

        let outcome = loop {
            if cancel.load(Ordering::Relaxed) {
                break Err(anyhow::Error::new(Failure::Cancelled));
            }
            let wire = match self.protocol.request(&self.endpoint, req) {
                Ok(wire) => wire,
                Err(e) => break Err(e),
            };
            let attempt_result = super::send(self, &wire).and_then(|reader| {
                let mut tracked = Tracked {
                    inner: out,
                    emitted: &mut emitted,
                };
                self.protocol.decode(reader, &mut tracked, cancel)
            });
            match attempt_result {
                Ok(message) => break Ok(message),
                Err(e) => {
                    let Some(failure) = e.downcast_ref::<Failure>() else {
                        break Err(e);
                    };
                    let cancelled = cancel.load(Ordering::Relaxed);
                    let class = retry::classify(failure, emitted, cancelled);
                    if class == Class::Fatal {
                        if !cancelled {
                            line.send(crate::server::agent::request::live::Element::Notice(
                                retry::notice(failure, self.retry.timeout),
                            ));
                        }
                        crate::server::platform::log::error(&self.model, format!("{e:#}"));
                        break Err(e);
                    }
                    line.send(crate::server::agent::request::live::Element::Notice(
                        retry::notice(failure, self.retry.timeout),
                    ));
                    attempt += 1;
                    crate::server::platform::log::warn(
                        &self.model,
                        format!(
                            "request failed: {e:#}; retrying {attempt}/{}",
                            self.retry.max_retries
                        ),
                    );
                    if attempt > self.retry.max_retries {
                        break Err(e);
                    }
                    self.replace_worker();
                    std::thread::sleep(self.retry.interval);
                }
            }
        };

        done.store(true, Ordering::Relaxed);
        let _ = watcher.join();
        outcome
    }
}

/// Counts whether anything reached the front end: after that, retrying would duplicate it.
struct Tracked<'a> {
    inner: &'a mut dyn Output,
    emitted: &'a mut bool,
}

impl Output for Tracked<'_> {
    fn stream(&mut self, piece: crate::server::agent::request::live::Stream) {
        *self.emitted = true;
        self.inner.stream(piece);
    }

    fn block(&mut self, block: crate::server::agent::request::contract::Block) {
        *self.emitted = true;
        self.inner.block(block);
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(timeout))
        .timeout_recv_body(Some(Duration::from_secs(1)))
        .http_status_as_error(false)
        .build()
        .into()
}

pub(crate) fn send_error(e: ureq::Error) -> anyhow::Error {
    anyhow::Error::new(Failure::NoResponse {
        source: e.to_string(),
    })
}
