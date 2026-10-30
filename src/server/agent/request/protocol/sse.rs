//! Server-sent events, line by line. Pure mechanics: no protocol knowledge.

use anyhow::Result;

use super::super::retry::Failure;

/// What one line turned out to be.
pub enum Frame {
    /// One event's payload, already stripped of the `data: ` prefix.
    Payload(String),
    /// The stream's end marker.
    Done,
    /// A blank line, a comment, an `event:` line: nothing a payload reader cares about.
    Skip,
}

/// Read one raw line.
pub fn read(reader: &mut dyn std::io::BufRead, buf: &mut Vec<u8>) -> std::io::Result<usize> {
    buf.clear();
    reader.read_until(b'\n', buf)
}

/// Classify one line. A worker failure travels inside the stream, so it surfaces here.
pub fn frame(line: &str) -> Result<Frame> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);

    if let Some(error) = line.strip_prefix("__WORKER_ERROR__") {
        if let Some(rest) = error.strip_prefix("__HTTP__") {
            let (status, body) = rest.split_once('\n').unwrap_or((rest, ""));
            if let Ok(status) = status.parse::<u16>() {
                return Err(anyhow::Error::new(Failure::Http {
                    status,
                    body: body.to_string(),
                }));
            }
        }
        return Err(anyhow::Error::new(Failure::NoResponse {
            source: error.to_string(),
        }));
    }

    let Some(payload) = line.strip_prefix("data: ") else {
        return Ok(Frame::Skip);
    };
    if payload == "[DONE]" {
        return Ok(Frame::Done);
    }
    Ok(Frame::Payload(payload.to_string()))
}
