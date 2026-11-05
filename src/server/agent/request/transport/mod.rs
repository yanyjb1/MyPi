//! Sending one call: the worker subprocess when it runs, a direct connection otherwise.

pub mod client;
pub mod worker;

use std::io::{BufRead, Read};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use anyhow::{Result, anyhow};

use self::client::{ChannelReader, Client};
use self::worker::SessionWorker;
use super::protocol::{ProtocolId, WireRequest};
use super::retry::Failure;

/// One provider as the configuration resolved it.
#[derive(Debug, Clone)]
pub struct Provider {
    pub name: String,
    pub base_url: String,
    pub protocol: ProtocolId,
    pub api_key: String,
    /// False sends no credential header at all.
    pub auth_header: bool,
}

pub(crate) fn send(
    client: &Client,
    worker: &Arc<SessionWorker>,
    wire: &WireRequest,
) -> Result<Box<dyn BufRead>> {
    if let Some(shared) = worker.request() {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let url = wire.url.clone();
        let body = wire.body.clone();
        let headers = wire.headers.clone();
        let timeout_ms = client.retry.timeout.as_millis() as u64;
        let slot = worker.clone();
        std::thread::spawn(move || {
            let result = shared
                .lock()
                .map_err(|_| anyhow!("HTTP worker lock poisoned"))
                .and_then(|mut w| {
                    w.stream(&url, body, &headers, timeout_ms, |line| {
                        tx.send(line.to_string()).is_ok()
                    })
                });
            slot.end();
            if let Err(e) = result {
                let _ = tx.send(format!("__WORKER_ERROR__{e}"));
            }
        });
        return Ok(Box::new(std::io::BufReader::new(ChannelReader {
            rx,
            pending: Vec::new(),
        })));
    }

    let mut req = client.http().post(&wire.url);
    for (name, value) in &wire.headers {
        req = req.header(name, value);
    }
    let resp = req.send_json(&wire.body).map_err(client::send_error)?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let body = resp
            .into_body()
            .read_to_string()
            .unwrap_or_else(|_| String::new());
        return Err(anyhow::Error::new(Failure::Http { status, body }));
    }
    let error = Arc::new(Mutex::new(None::<String>));
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let mut body = resp.into_body().into_reader();
    let why = error.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match body.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    if let Ok(mut slot) = why.lock() {
                        *slot = Some(format!("{e:#}"));
                    }
                    break;
                }
            }
        }
    });
    Ok(Box::new(std::io::BufReader::new(LineReader {
        rx,
        pending: Vec::new(),
        eof: false,
        error,
    })))
}

/// How long one read waits before reporting silence; the decoder owns the real deadline.
const POLL: Duration = Duration::from_secs(1);

/// A body read on its own thread, handed over one whole line at a time.
///
/// A poll that finds half a line keeps it and reports silence, so no layer above ever sees a
/// split line, and a stream's only deadline is the decoder's silence budget.
pub(crate) struct LineReader {
    rx: mpsc::Receiver<Vec<u8>>,
    pending: Vec<u8>,
    eof: bool,
    error: Arc<Mutex<Option<String>>>,
}

impl std::io::Read for LineReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while !self.pending.contains(&b'\n') && !self.eof {
            match self.rx.recv_timeout(POLL) {
                Ok(chunk) => self.pending.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "no line yet",
                    ));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.eof = true;
                }
            }
        }
        if self.eof
            && let Ok(mut slot) = self.error.lock()
            && let Some(text) = slot.take()
        {
            return Err(std::io::Error::other(text));
        }
        if self.pending.is_empty() {
            return Ok(0);
        }
        let n = out.len().min(self.pending.len());
        out[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}
