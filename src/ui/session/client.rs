//! The wire: one connection, one queue of frames, one table of frames still owed an answer.

use std::collections::VecDeque;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::ipc::client::Client as Socket;
use crate::ipc::contract::Frame;
use crate::ipc::endpoint::Endpoint;

/// What came back: an unsolicited frame, the answer to a queued one, or a queue that broke.
#[derive(Debug)]
pub enum Incoming {
    Push(Frame),
    Answer { request: Frame, reply: Frame },
    Lost { request: Frame, reason: String },
}

/// The connection, the queue, and every frame already sent.
pub struct Client {
    socket: Socket,
    queue: VecDeque<(Frame, bool)>,
    waiting: VecDeque<Frame>,
}

impl Client {
    /// Connect and register. Who starts the server is not this layer's business.
    pub fn connect(endpoint: Endpoint, viewer: Value) -> Result<Client> {
        let socket = Socket::connect(&endpoint, &viewer)
            .with_context(|| format!("cannot reach the server at {endpoint:?}"))?;
        Ok(Client {
            socket,
            queue: VecDeque::new(),
            waiting: VecDeque::new(),
        })
    }

    /// Queue one frame; `answer` says whether the server replies to it.
    pub fn queue(&mut self, frame: Frame, answer: bool) {
        self.queue.push_back((frame, answer));
    }

    /// Drop everything not yet sent.
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// Still connected.
    pub fn alive(&self) -> bool {
        self.socket.alive()
    }

    /// Take what arrived, send what is queued, take what that earned.
    pub fn pump(&mut self) -> Vec<Incoming> {
        let mut out = Vec::new();
        self.receive(&mut out);
        while let Some((frame, answer)) = self.queue.pop_front() {
            match self.socket.send(frame.clone()) {
                Ok(()) => {
                    if answer {
                        self.waiting.push_back(frame);
                    }
                }
                Err(e) => out.push(Incoming::Lost {
                    request: frame,
                    reason: format!("{e:#}"),
                }),
            }
        }
        self.receive(&mut out);
        out
    }

    fn receive(&mut self, out: &mut Vec<Incoming>) {
        for frame in self.socket.drain() {
            if frame.kind == "stream" {
                out.push(Incoming::Push(frame));
                continue;
            }
            loop {
                let Some(head) = self.waiting.front() else {
                    break;
                };
                let matched = head.kind == frame.kind && head.name == frame.name;
                let request = self.waiting.pop_front().expect("the head was just read");
                if matched {
                    out.push(Incoming::Answer {
                        request,
                        reply: frame,
                    });
                    break;
                }
                out.push(Incoming::Lost {
                    request,
                    reason: format!("unexpected reply: {} {}", frame.kind, frame.name),
                });
            }
        }
    }
}
