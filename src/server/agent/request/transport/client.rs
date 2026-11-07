//! One provider connection: its protocol, its retry policy, and the child's output.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;

use crate::server::agent::request::live::Line;
use crate::server::agent::request::message::AssistantMessage;
use crate::server::agent::request::protocol::stream::Budget;
use crate::server::agent::request::protocol::{Endpoint, Output, Protocol, ProtocolRequest};
use crate::server::agent::request::retry::{self, Class, Failure, RetryPolicy};
use crate::server::agent::request::transport::Provider;
use crate::server::agent::request::transport::worker::SessionWorker;

/// One provider, one model, one protocol instance.
pub struct Client {
    endpoint: Endpoint,
    protocol: Box<dyn Protocol>,
    model: String,
    pub(crate) retry: RetryPolicy,
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
            retry,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// The deadlines the child holds a silent stream to.
    pub(crate) fn budget(&self) -> Budget {
        self.protocol.budget()
    }

    /// One call, retried per policy. Every failure is said out loud exactly once.
    pub fn complete(
        &self,
        req: &ProtocolRequest,
        out: &mut dyn Output,
        line: &Line,
        cancel: &AtomicBool,
        worker: &std::sync::Arc<SessionWorker>,
    ) -> Result<AssistantMessage> {
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
            let attempt_result = super::send(self, worker, &wire).and_then(|reader| {
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
                    if cancel.load(Ordering::Relaxed) {
                        break Err(anyhow::Error::new(Failure::Cancelled));
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
                    worker.restart();
                    if !wait(cancel, self.retry.interval) {
                        break Err(anyhow::Error::new(Failure::Cancelled));
                    }
                }
            }
        };

        outcome
    }
}

/// Sleep in slices so an interrupt is heard before the interval ends.
fn wait(cancel: &AtomicBool, total: Duration) -> bool {
    const STEP: Duration = Duration::from_millis(50);
    let mut left = total;
    while !left.is_zero() {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        let slice = left.min(STEP);
        std::thread::sleep(slice);
        left = left.saturating_sub(slice);
    }
    !cancel.load(Ordering::Relaxed)
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
