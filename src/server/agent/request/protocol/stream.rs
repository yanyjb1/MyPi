//! One SSE stream: read it, and say whether the turn was cancelled.

use crate::platform::stop::Stop;
use std::io::BufRead;
use std::time::Duration;

use anyhow::Context as _;

use super::sse::{self, Frame};

/// The longest a provider may stay silent, before the first payload and between two.
pub const SILENCE: Duration = Duration::from_secs(600);

/// Read `data:` payloads until the stream ends; `take` returns `false` to stop reading.
/// The answer is true only when the turn was cancelled rather than the stream ending.
pub fn data(
    reader: &mut dyn BufRead,
    cancel: &Stop,
    mut take: impl FnMut(&str) -> anyhow::Result<bool>,
) -> anyhow::Result<bool> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match sse::read(reader, &mut buf) {
            Ok(0) => return Ok(cancel.is_stopped()),
            Ok(_) => {}
            Err(e) => {
                if cancel.is_stopped() {
                    return Ok(true);
                }
                return Err(e).context("stream read interrupted");
            }
        }

        let payload = match sse::frame(&String::from_utf8_lossy(&buf))? {
            Frame::Skip => continue,
            Frame::Done => return Ok(false),
            Frame::Payload(payload) => payload,
        };
        if !take(&payload)? {
            return Ok(false);
        }
    }
}
