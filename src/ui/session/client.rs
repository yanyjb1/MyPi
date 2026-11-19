// The wire: one connection, one queue of frames, one table of frames still owed an answer.

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

// The connection, the queue, and every frame already sent.
pub struct Client {
    socket: Socket,
    queue: VecDeque<(Frame, bool)>,
    waiting: VecDeque<Frame>,
}

impl Client {
    // Connect and register. Who starts the server is not this layer's business.
    pub fn connect(endpoint: Endpoint, viewer: Value) -> Result<Client> {
        let socket = Socket::connect(&endpoint, &viewer)
            .with_context(|| format!("cannot reach the server at {endpoint:?}"))?;
        Ok(Client {
            socket,
            queue: VecDeque::new(),
            waiting: VecDeque::new(),
        })
    }

    // Queue one frame; `answer` says whether the server replies to it.
    pub fn queue(&mut self, frame: Frame, answer: bool) {
        self.queue.push_back((frame, answer));
    }

    // Drop everything not yet sent or not yet answered.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.waiting.clear();
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
        for frame in self.socket.drain() {
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
            }
        }
    }
}
