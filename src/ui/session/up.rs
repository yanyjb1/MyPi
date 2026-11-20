// The upstream half: one IPC connection, frames out and frames in.

use anyhow::Result;
use serde_json::Value;

use crate::ipc::client::Client;
use crate::ipc::contract::Frame;
use crate::ipc::endpoint::Endpoint;

pub struct Up {
    client: Client,
}

impl Up {
    pub fn connect(endpoint: Endpoint) -> Result<Up> {
        Ok(Up {
            client: Client::connect(&endpoint, &viewer())?,
        })
    }

    // A write that fails leaves the read half to report the death.
    pub fn send(&self, frame: Frame) {
        let _ = self.client.send(frame);
    }

    pub fn drain(&self) -> Vec<Frame> {
        self.client.drain()
    }

    pub fn alive(&self) -> bool {
        self.client.alive()
    }
}

fn viewer() -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    Value::String(format!("{}-{now}", std::process::id()))
}
