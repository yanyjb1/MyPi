//! Sending one call: the worker subprocess when it runs, a direct connection otherwise.

pub mod client;
pub mod worker;

use std::io::BufRead;

use anyhow::{Result, anyhow};

use self::client::{ChannelReader, Client};
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

pub(crate) fn send(client: &Client, wire: &WireRequest) -> Result<Box<dyn BufRead>> {
    let worker = client.worker();
    if let Some(worker) = worker {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let url = wire.url.clone();
        let body = wire.body.clone();
        let headers = wire.headers.clone();
        let timeout_ms = client.retry.timeout.as_millis() as u64;
        std::thread::spawn(move || {
            let result = worker
                .lock()
                .map_err(|_| anyhow!("HTTP worker lock poisoned"))
                .and_then(|mut w| {
                    w.stream(&url, body, &headers, timeout_ms, |line| {
                        tx.send(line.to_string()).is_ok()
                    })
                });
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
    Ok(Box::new(std::io::BufReader::new(
        resp.into_body().into_reader(),
    )))
}
