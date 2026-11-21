//! `data:` frames: the payload, the end marker, or nothing.

use anyhow::Result;

use crate::server::agent::request::retry::Failure;

/// What one line turned out to be.
pub enum Frame {
    /// One event's payload, already stripped of the `data: ` prefix.
    Payload(String),
    /// The stream's end marker.
    Done,
    /// A blank line, a comment, an `event:` line: nothing a payload reader cares about.
    Skip,
}

/// A worker failure travels inside the stream; classify it before anything else.
pub fn worker_error(line: &str) -> Option<anyhow::Error> {
    let line = line.trim_end_matches(['\r', '\n']);
    let error = line.strip_prefix("__WORKER_ERROR__")?;
    if let Some(rest) = error.strip_prefix("__HTTP__") {
        let (status, body) = rest.split_once('\n').unwrap_or((rest, ""));
        if let Ok(status) = status.parse::<u16>() {
            return Some(anyhow::Error::new(Failure::Http {
                status,
                body: body.to_string(),
            }));
        }
    }
    Some(anyhow::Error::new(Failure::NoResponse {
        source: error.to_string(),
    }))
}

/// One field's value: the name, a colon, and at most one space SSE leaves optional.
pub fn strip_field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(name)?.strip_prefix(':')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// Classify one line.
pub fn frame(line: &str) -> Result<Frame> {
    if let Some(error) = worker_error(line) {
        return Err(error);
    }
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);

    let Some(payload) = strip_field(line, "data") else {
        return Ok(Frame::Skip);
    };
    if payload == "[DONE]" {
        return Ok(Frame::Done);
    }
    Ok(Frame::Payload(payload.to_string()))
}
