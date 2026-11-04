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
    loop {
        match sse::read(reader, &mut buf) {
            Ok(0) => {
                return Ok(if cancel.load(Ordering::Relaxed) {
                    End::Canceled
                } else {
                    End::Eof
                });
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(End::Canceled);
                }
                if !got_first
                    && let Some(budget) = budget.first
                    && started.elapsed() > budget
                {
                    anyhow::bail!("no first event from upstream within {budget:?}");
                }
                if got_first
                    && let Some(budget) = budget.idle
                    && last.elapsed() > budget
                {
                    anyhow::bail!("no new event from upstream within {budget:?}");
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
        last = Instant::now();
        if !take(&payload)? {
            return Ok(End::Done);
        }
    }
}

/// Read `event:` / `data:` frames until the stream ends; `take` returns `false` to stop reading.
pub fn events(
    reader: &mut dyn BufRead,
    cancel: &AtomicBool,
    mut take: impl FnMut(&SseEvent) -> anyhow::Result<bool>,
) -> anyhow::Result<End> {
    while let Some(event) = sse::read_event(reader)? {
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
