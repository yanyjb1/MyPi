//! Sending one call through the session's worker child.

pub mod client;
pub mod worker;

use std::io::BufRead;
use std::sync::Arc;

use anyhow::{Result, anyhow};

use self::client::Client;
use self::worker::{ChannelReader, SessionWorker};
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

/// One request, on the child that owns the session's sockets.
pub(crate) fn send(
    client: &Client,
    worker: &Arc<SessionWorker>,
    wire: &WireRequest,
) -> Result<Box<dyn BufRead>> {
    let shared = worker.request().ok_or_else(|| {
        anyhow::Error::new(Failure::NoResponse {
            source: "the HTTP worker did not start".into(),
        })
    })?;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let url = wire.url.clone();
    let body = wire.body.clone();
    let headers = wire.headers.clone();
    let timeout_ms = client.retry.timeout.as_millis() as u64;
    let (first, idle) = client.budget().resolved();
    let first_ms = first.as_millis() as u64;
    let idle_ms = idle.as_millis() as u64;
    std::thread::spawn(move || {
        let result = shared
            .lock()
            .map_err(|_| anyhow!("HTTP worker lock poisoned"))
            .and_then(|mut w| {
                w.stream(
                    &url,
                    body,
                    &headers,
                    timeout_ms,
                    first_ms,
                    idle_ms,
                    |line| tx.send(line.to_string()).is_ok(),
                )
            });
        if let Err(e) = result {
            let _ = tx.send(format!("__WORKER_ERROR__{e}"));
        }
    });
    Ok(Box::new(std::io::BufReader::new(ChannelReader {
        rx,
        pending: Vec::new(),
    })))
}
