//! The one IPC client: connect, the request queue, the watch, the upstream loop.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, TryRecvError};

use anyhow::{Context, Result};

use crate::ipc::client::Client as Door;
use crate::ipc::contract::{Call, Event, Opened, Post, PostDone, Reply, Subscribe, Viewer};
use crate::ipc::endpoint::Endpoint;

/// One queued request: the session's translation of a front end call.
#[derive(Debug, Clone)]
pub enum Request {
    Call(Call),
    Post(Post),
    Subscribe(Subscribe),
    /// Stop the watch; dropping the stream closes it.
    Unwatch,
    Bye,
}

/// Which stretch a round-opening subscription belonged to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Round {
    Submit,
    Compact,
}

/// What kind of request failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Read,
    Change,
    Round,
    Watch,
}

/// What came back: the upstream object, and the local facts about the connection itself.
#[derive(Debug)]
pub enum Arrival {
    Stream(Event),
    /// An answer with the question it answers.
    Answer {
        call: Call,
        reply: Reply,
    },
    Done(PostDone),
    Opened {
        session: i64,
        round: Round,
        opened: Opened,
    },
    /// The watch ended: the server closed the stream, or the connection died.
    WatchEnded,
    /// The door failed. The connection stays usable.
    Failed {
        kind: Kind,
        call: Option<Call>,
        message: String,
    },
}

/// The connection, the queue, and the watch. Nothing is sent until the next `pump`.
pub struct Client {
    door: Door,
    queue: VecDeque<Request>,
    watch: Option<Receiver<Event>>,
}

impl Client {
    /// Connect and register. Who starts the server is not this layer's business.
    pub fn connect(endpoint: Endpoint, viewer: Viewer) -> Result<Client> {
        let door = Door::connect(endpoint.clone(), viewer)
            .with_context(|| format!("cannot reach the server at {endpoint:?}"))?;
        Ok(Client {
            door,
            queue: VecDeque::new(),
            watch: None,
        })
    }

    pub fn queue(&mut self, request: Request) {
        self.queue.push_back(request);
    }

    /// Drop everything not yet sent.
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// Take what the stream pushed, send what is queued, take what that earned.
    pub fn pump(&mut self) -> Vec<Arrival> {
        let mut out = Vec::new();
        self.drain_stream(&mut out);
        while let Some(request) = self.queue.pop_front() {
            let kind = kind_of(&request);
            let call = match &request {
                Request::Call(call) => Some(call.clone()),
                _ => None,
            };
            match self.send(request) {
                Ok(Some(arrival)) => out.push(arrival),
                Ok(None) => {}
                Err(e) => out.push(Arrival::Failed {
                    kind,
                    call,
                    message: format!("{e:#}"),
                }),
            }
        }
        self.drain_stream(&mut out);
        out
    }

    fn drain_stream(&mut self, out: &mut Vec<Arrival>) {
        let Some(rx) = self.watch.take() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(item) => out.push(Arrival::Stream(item)),
                Err(TryRecvError::Empty) => {
                    self.watch = Some(rx);
                    break;
                }
                Err(TryRecvError::Disconnected) => {
                    out.push(Arrival::WatchEnded);
                    break;
                }
            }
        }
    }

    /// The only place an upstream wire call is made.
    fn send(&mut self, request: Request) -> Result<Option<Arrival>> {
        Ok(match request {
            Request::Call(call) => {
                let reply = self.door.call(call.clone())?;
                Some(Arrival::Answer { call, reply })
            }
            Request::Post(post) => Some(Arrival::Done(self.door.post(post)?)),
            Request::Subscribe(subscribe) => match subscribe {
                Subscribe::Watch { session } => {
                    let stream = self.door.watch(session)?;
                    self.watch = Some(stream);
                    None
                }
                Subscribe::Submit {
                    session,
                    prefix,
                    text,
                    suffix,
                } => Some(Arrival::Opened {
                    session,
                    round: Round::Submit,
                    opened: self.door.submit(
                        session,
                        prefix.as_deref(),
                        &text,
                        suffix.as_deref(),
                    )?,
                }),
                Subscribe::Compact {
                    session,
                    retention,
                    focus,
                } => Some(Arrival::Opened {
                    session,
                    round: Round::Compact,
                    opened: self.door.compact(session, retention, focus)?,
                }),
            },
            Request::Unwatch => {
                self.watch = None;
                None
            }
            Request::Bye => {
                self.door.bye();
                None
            }
        })
    }
}

fn kind_of(request: &Request) -> Kind {
    match request {
        Request::Call(_) => Kind::Read,
        Request::Post(_) => Kind::Change,
        Request::Subscribe(Subscribe::Watch { .. }) => Kind::Watch,
        Request::Subscribe(_) => Kind::Round,
        Request::Unwatch => Kind::Watch,
        Request::Bye => Kind::Change,
    }
}
