//! The inbound half: send once, decode as it arrives. Retry lives here — a stream is only retried
//! while nothing has been emitted, and an interrupt is never retried.

pub mod decode;
pub mod inband;
pub mod markup;
pub mod response;

use std::sync::atomic::{AtomicBool, Ordering};

use crate::server::agent::request::assembly;
use crate::server::agent::request::content::AnswerSink;
use crate::server::agent::request::message::{AssistantMessage, Message, ToolDef};
use crate::server::agent::request::transport;
use crate::server::agent::request::transport::client::{Client, is_strict_rejection};

/// One streaming request: decode as it arrives, hand content to `sink`.
pub(crate) fn streamed(
    client: &Client,
    system: &str,
    stored: &[Message],
    tools: &[ToolDef],
    max_tokens: u32,
    cancel: &AtomicBool,
    sink: &mut dyn AnswerSink,
) -> anyhow::Result<AssistantMessage> {
    let emitted = std::cell::Cell::new(false);
    let mut n = 1;
    // The host refused a strict tool schema: retry once without strict.
    let mut no_strict = false;
    loop {
        // A copy with `no_strict` set, rather than another `body_json` parameter: one place decides.
        let mut compat = client.compat.clone();
        if no_strict {
            compat.supports_strict_mode = false;
        }
        let body = assembly::build(
            &client.model,
            max_tokens,
            true,
            system,
            stored,
            tools,
            &compat,
            client.session_id.as_deref(),
        );
        let r = transport::send(client, &body).and_then(|reader| {
            let mut tracked = Tracked {
                inner: &mut *sink,
                emitted: &emitted,
            };
            decode::streamed(reader, client, cancel, &mut tracked)
        });
        match r {
            Ok(msg) => {
                if n > 1 {
                    crate::server::platform::log::info(&client.model, format!("attempt {n} succeeded"));
                }
                return Ok(msg);
            }
            Err(e) if !emitted.get() && !no_strict && is_strict_rejection(&e) => {
                crate::server::platform::log::warn(
                    &client.model,
                    format!("host rejected the strict tool schema; retrying once without strict: {e:#}"),
                );
                no_strict = true;
                n += 1;
                continue;
            }
            Err(e) => {
                // Text is already streaming: never resend.
                let again = !emitted.get()
                    && !cancel.load(Ordering::Relaxed)
                    && client.should_retry(n, &e);
                if !again {
                    return Err(e);
                }
                std::thread::sleep(std::time::Duration::from_millis(client.retry.delay_ms(n)));
                n += 1;
            }
        }
    }
}

/// Wraps a sink to remember whether anything was emitted.
struct Tracked<'a> {
    inner: &'a mut dyn AnswerSink,
    emitted: &'a std::cell::Cell<bool>,
}

impl AnswerSink for Tracked<'_> {
    fn content(&mut self, text: &str) -> bool {
        self.emitted.set(true);
        self.inner.content(text)
    }

    fn reasoning(&mut self, text: &str) -> bool {
        self.emitted.set(true);
        self.inner.reasoning(text)
    }
}
