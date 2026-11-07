//! One SSE stream: read, cancel, and how it ended.

use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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

/// How long the stream may stay silent; the worker child enforces it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Budget {
    /// Until the first payload arrives.
    pub first: Option<Duration>,
    /// Between payloads, after the first one.
    pub idle: Option<Duration>,
}

impl Budget {
    /// The concrete deadlines; a host that names none gets the silence default.
    pub fn resolved(&self) -> (Duration, Duration) {
        (self.first.unwrap_or(SILENCE), self.idle.unwrap_or(SILENCE))
    }
}

/// Read `data:` payloads until the stream ends; `take` returns `false` to stop reading.
pub fn data(
    reader: &mut dyn BufRead,
    cancel: &AtomicBool,
    mut take: impl FnMut(&str) -> anyhow::Result<bool>,
) -> anyhow::Result<End> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match sse::read(reader, &mut buf) {
            Ok(0) => return Ok(ended(cancel)),
            Ok(_) => {}
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
    loop {
        let event = match sse::read_event(reader) {
            Ok(event) => event,
            Err(e) => {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(End::Canceled);
                }
                return Err(e).context("stream read interrupted");
            }
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
    Ok(ended(cancel))
}

/// The longest a provider may stay silent when it names no budget of its own.
const SILENCE: Duration = Duration::from_secs(600);

fn ended(cancel: &AtomicBool) -> End {
    if cancel.load(Ordering::Relaxed) {
        End::Canceled
    } else {
        End::Eof
    }
}
