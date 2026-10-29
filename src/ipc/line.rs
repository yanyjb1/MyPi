//! One message, one line: a newline ends each JSON value, and a value never
//! holds a literal newline. `MAX_LINE_BYTES` caps a runaway line.

use std::io::{self, BufRead};

/// One line may not be longer than this. A longer one is a protocol error, not
/// a reason to keep growing a buffer.
pub const MAX_LINE_BYTES: usize = 32 * 1024 * 1024;

/// A value as one line: its JSON, then the newline that ends it.
pub fn encode<T: serde::Serialize>(value: &T) -> io::Result<Vec<u8>> {
    let mut out =
        serde_json::to_vec(value).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    out.push(b'\n');
    Ok(out)
}

/// One line as a value. The newline may still be on the end.
pub fn decode<T: serde::de::DeserializeOwned>(line: &[u8]) -> io::Result<T> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    serde_json::from_slice(line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Where the last scan stopped, so a growing line is not rescanned from the
/// start on every chunk.
pub struct Scanner {
    from: usize,
}

impl Scanner {
    pub fn new() -> Scanner {
        Scanner { from: 0 }
    }

    /// The length up to and including the next newline, when the buffer has
    /// one. Otherwise `None`, and the next call resumes where this one stopped.
    pub fn next(&mut self, buf: &[u8]) -> Option<usize> {
        if self.from >= buf.len() {
            self.from = buf.len();
            return None;
        }
        match buf[self.from..].iter().position(|&b| b == b'\n') {
            Some(at) => {
                let end = self.from + at + 1;
                self.from = 0;
                Some(end)
            }
            None => {
                self.from = buf.len();
                None
            }
        }
    }

    /// Start over: the bytes it has seen are gone.
    pub fn reset(&mut self) {
        self.from = 0;
    }
}

impl Default for Scanner {
    fn default() -> Self {
        Scanner::new()
    }
}

/// One connection's read face: bytes in, whole lines out. Bytes past an end of
/// line stay for the next call.
pub struct Lines<R: BufRead> {
    reader: R,
    scanner: Scanner,
    buf: Vec<u8>,
    /// How much of `buf` the last line took; dropped at the next call.
    taken: usize,
}

impl<R: BufRead> Lines<R> {
    pub fn new(reader: R) -> Lines<R> {
        Lines {
            reader,
            scanner: Scanner::new(),
            buf: Vec::new(),
            taken: 0,
        }
    }

    /// The next whole line, newline included; `None` when the peer closed
    /// between messages. A close in the middle of a line is an error.
    pub fn next_line(&mut self) -> io::Result<Option<&[u8]>> {
        if self.taken > 0 {
            self.buf.drain(..self.taken);
            self.taken = 0;
            self.scanner.reset();
        }
        loop {
            if let Some(end) = self.scanner.next(&self.buf) {
                self.taken = end;
                return Ok(Some(&self.buf[..end]));
            }
            if self.buf.len() > MAX_LINE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "line exceeds MAX_LINE_BYTES",
                ));
            }
            let chunk = self.reader.fill_buf()?;
            if chunk.is_empty() {
                return if self.buf.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed in the middle of a line",
                    ))
                };
            }
            // Take no more than the cap: one byte over is what tells a runaway
            // line apart from a big message.
            let room = MAX_LINE_BYTES + 1 - self.buf.len();
            let take = chunk.len().min(room);
            self.buf.extend_from_slice(&chunk[..take]);
            self.reader.consume(take);
        }
    }
}
