//! One SSE stream: read, cancel, the silent-connection budgets, and how it ended.

use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context as _;

use super::sse::{self, Frame, SseEvent};

/// What ended the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// The upstream closed the connection.
    Eof,
    /// The upstream sent its end marker.
    Done,
    /// The turn was cancelled.
    Canceled,
}

/// How long the stream may stay silent.
#[derive(Debug, Clone, Copy, Default)]
pub struct Budget {
    /// Until the first payload arrives.
    pub first: Option<Duration>,
    /// Between payloads, after the first one.
    pub idle: Option<Duration>,
}

/// Read `data:` payloads until the stream ends; `take` returns `false` to stop reading.
pub fn data(
    reader: &mut dyn BufRead,
    cancel: &AtomicBool,
    budget: Budget,
    mut take: impl FnMut(&str) -> anyhow::Result<bool>,
) -> anyhow::Result<End> {
    let mut buf: Vec<u8> = Vec::new();
    let started = Instant::now();
    let mut last = Instant::now();
    let mut got_first = false;
    let first = budget.first.unwrap_or(SILENCE);
    let idle = budget.idle.unwrap_or(SILENCE);
    loop {
        match sse::read(reader, &mut buf) {
            Ok(0) => {
                return Ok(if cancel.load(Ordering::Relaxed) {
                    End::Canceled
                } else {
                    End::Eof
                });
            }
            Ok(_) => last = Instant::now(),
            Err(e) if silent_io(&e) => {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(End::Canceled);
                }
                if !got_first && started.elapsed() > first {
                    anyhow::bail!("no first event from upstream within {first:?}");
                }
                if got_first && last.elapsed() > idle {
                    anyhow::bail!("no new event from upstream within {idle:?}");
                }
                continue;
            }
            Err(e) => {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(End::Canceled);
                }
                return Err(e).context("stream read interrupted");
            }
        }

        let payload = match sse::frame(&String::from_utf8_lossy(&buf))? {
            Frame::Skip => continue,
            Frame::Done => return Ok(End::Done),
            Frame::Payload(payload) => payload,
        };
        got_first = true;
        if !take(&payload)? {
            return Ok(End::Done);
        }
    }
}

/// The longest a provider may stay silent when it names no budget of its own.
const SILENCE: Duration = Duration::from_secs(600);

/// A read that timed out is not a failure: the reader polls so cancellation stays possible.
fn silent_io(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::TimedOut || e.to_string().contains("timeout")
}

/// The same test through an `anyhow` chain: a wrapped timeout is still a timeout.
fn silent(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(silent_io)
            || cause.to_string().contains("timeout")
    })
}

/// Read `event:` / `data:` frames until the stream ends; `take` returns `false` to stop reading.
pub fn events(
    reader: &mut dyn BufRead,
    cancel: &AtomicBool,
    mut take: impl FnMut(&SseEvent) -> anyhow::Result<bool>,
) -> anyhow::Result<End> {
    loop {
        let event = match sse::read_event(reader) {
            Ok(event) => event,
            Err(e) if silent(&e) => {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(End::Canceled);
                }
                continue;
            }
            Err(e) => return Err(e).context("stream read interrupted"),
        };
        let Some(event) = event else {
            break;
        };
        if cancel.load(Ordering::Relaxed) {
            return Ok(End::Canceled);
        }
        if !take(&event)? {
            return Ok(End::Done);
        }
    }
    Ok(if cancel.load(Ordering::Relaxed) {
        End::Canceled
    } else {
        End::Eof
    })
}
