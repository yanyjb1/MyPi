//! Transport: send a request body out, get the response's **lines** back. In the subprocess when
//! there is one (an interrupt kills it), direct otherwise.

pub mod client;
pub mod worker;

use std::io::BufRead;

use anyhow::anyhow;
use serde_json::Value;

use crate::server::agent::request::transport::client::{ChannelReader, Client};
/// Send a request body out and hand the response back as a line source.
pub(crate) fn send(client: &Client, body: &Value) -> anyhow::Result<Box<dyn BufRead>> {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let worker_slot = client.worker.lock().ok().and_then(|slot| slot.clone());
    let worker_used = worker_slot.is_some();
    if let Some(worker_slot) = worker_slot {
        let base = client.base_url.clone();
        let key = client.api_key.clone();
        let request_body = body.clone();
        let headers = client.extra_headers();
        std::thread::spawn(move || {
            let result = worker_slot.lock().map_err(|_| anyhow!("HTTP worker lock poisoned")).and_then(|mut w| {
                w.stream(&base, &key, request_body, &headers, |line| tx.send(line.to_string()).is_ok())
            });
            if let Err(e) = result { let _ = tx.send(format!("__WORKER_ERROR__{e}")); }
        });
    }
    let reader: Box<dyn BufRead> = if worker_used {
        Box::new(std::io::BufReader::new(ChannelReader { rx, pending: Vec::new() }))
    } else {
        Box::new(std::io::BufReader::new(
            client.post(&client.http, body)?.into_body().into_reader(),
        ))
    };
    Ok(reader)
}
