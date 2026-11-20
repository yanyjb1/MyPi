// The wire: one connection, its queue, its unanswered frames, and the opening word a clear keeps.

use std::collections::VecDeque;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::ipc::client::Client as Socket;
use crate::ipc::contract::Frame;
use crate::ipc::endpoint::Endpoint;

// What came back: an unsolicited frame or the answer to a queued one.
#[derive(Debug)]
pub enum Incoming {
    Push(Frame),
    Answer { request: Frame, reply: Frame },
}

// The connection, what waits to go out, and what went out still owed an answer.
pub struct Client {
    socket: Socket,
    queue: VecDeque<(Frame, bool)>,
    waiting: VecDeque<Frame>,
    // The server's opening word, read before the first attach; a clear keeps it.
    kept: Vec<Frame>,
}

impl Client {
    // Connect and register. Who starts the server is not this layer's business.
    pub fn connect(endpoint: Endpoint) -> Result<Client> {
        let socket = Socket::connect(&endpoint, &viewer())
            .with_context(|| format!("cannot reach the server at {endpoint:?}"))?;
        Ok(Client {
            socket,
            queue: VecDeque::new(),
            waiting: VecDeque::new(),
            kept: Vec::new(),
        })
    }

    // Queue one frame; `answer` says whether the server replies to it.
    pub fn queue(&mut self, frame: Frame, answer: bool) {
        self.queue.push_back((frame, answer));
    }

    // Drop everything not yet sent, not yet answered, and already read; the connection's opening word survives.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.waiting.clear();
        self.kept = self
            .socket
            .drain()
            .into_iter()
            .filter(|frame| frame.kind == "local" && frame.name == "opened")
            .collect();
    }

    // Still connected.
    pub fn alive(&self) -> bool {
        self.socket.alive()
    }

    // Take what arrived, send what is queued, take what that earned.
    pub fn pump(&mut self) -> Vec<Incoming> {
        let mut out = Vec::new();
        self.receive(&mut out);
        while let Some((frame, answer)) = self.queue.pop_front() {
            if self.socket.send(frame.clone()).is_ok() && answer {
                self.waiting.push_back(frame);
            }
        }
        self.receive(&mut out);
        out
    }

    fn receive(&mut self, out: &mut Vec<Incoming>) {
        let kept = std::mem::take(&mut self.kept);
        for frame in kept.into_iter().chain(self.socket.drain()) {
            if frame.kind == "stream" || frame.kind == "local" {
                out.push(Incoming::Push(frame));
                continue;
            }
            if self
                .waiting
                .front()
                .is_some_and(|request| request.kind == frame.kind && request.name == frame.name)
                && let Some(request) = self.waiting.pop_front()
            {
                out.push(Incoming::Answer {
                    request,
                    reply: frame,
                });
            } else {
                out.push(Incoming::Push(frame));
            }
        }
    }
}

// One identity for this front end instance.
fn viewer() -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    Value::String(format!("{}-{now}", std::process::id()))
}
