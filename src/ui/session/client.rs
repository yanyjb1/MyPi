//! The backend half: the connection, watching a session, and the request queue.
//! Design: `README.md`.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};

use anyhow::{Context, Result};

use crate::ipc::client::Client as Door;
use crate::ipc::viewer as make_viewer;
use crate::server::api::subscribe::Retention;
use crate::server::api::{Call, Opened, Post, PostDone, Reply, StreamItem};
use crate::server::platform::ipc::Cost;

/// What the front end asks the backend for. One queue holds them all, in order.
#[derive(Debug, Clone)]
pub enum Request {
    Call(Call),
    /// A change of session state, already applied locally.
    Post(Post),
    Submit {
        session: i64,
        prefix: Option<String>,
        text: String,
        suffix: Option<String>,
    },
    Compact {
        session: i64,
        retention: Retention,
        focus: Option<String>,
    },
    /// Watch a session: a stream connection of its own.
    Watch(i64),
    /// Stop watching. Dropping the channel closes the stream, which is the whole of it.
    Unwatch,
    Bye,
}

/// Which stretch a round-opening request belonged to.
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

/// What came back, in the shape the ingest side eats.
#[derive(Debug)]
pub enum Arrival {
    Stream(StreamItem),
    /// An answer, with the question it answers: a viewport page means what was asked.
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
        message: String,
    },
}

pub struct Client {
    door: Door,
    queue: VecDeque<Request>,
    watch: Option<Receiver<StreamItem>>,
}

impl Client {
    /// Connect and register. Who starts the server is not this layer's business.
    pub fn connect(cost: Cost, path: &Path) -> Result<Client> {
        let viewer = make_viewer();
        let door = Door::connect(cost, path, viewer)
            .with_context(|| format!("cannot reach the server at {}", path.display()))?;
        Ok(Client {
            door,
            queue: VecDeque::new(),
            watch: None,
        })
    }

    /// Ask for something. Nothing is sent until the next `pump`.
    pub fn queue(&mut self, request: Request) {
        self.queue.push_back(request);
    }

    /// Watch a session: the stream replaces this viewer's old one.
    pub fn watch(&mut self, session: i64) -> Result<()> {
        let stream = self.door.watch(session)?;
        self.watch = Some(stream);
        Ok(())
    }

    pub fn unwatch(&mut self) {
        self.watch = None;
    }

    pub fn bye(&self) {
        self.door.bye();
    }

    /// Take what the stream pushed, send what is queued, take what that earned.
    pub fn pump(&mut self) -> Vec<Arrival> {
        let mut out = Vec::new();
        self.drain_stream(&mut out);
        while let Some(request) = self.queue.pop_front() {
            let kind = kind_of(&request);
            match self.send(request) {
                Ok(Some(arrival)) => out.push(arrival),
                Ok(None) => {}
                Err(e) => out.push(Arrival::Failed {
                    kind,
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

    fn send(&mut self, request: Request) -> Result<Option<Arrival>> {
        Ok(match request {
            Request::Call(call) => {
                let reply = self.door.call(call.clone())?;
                Some(Arrival::Answer { call, reply })
            }
            Request::Post(post) => Some(Arrival::Done(self.door.post(post)?)),
            Request::Submit {
                session,
                prefix,
                text,
                suffix,
            } => Some(Arrival::Opened {
                session,
                round: Round::Submit,
                opened: self
                    .door
                    .submit(session, prefix.as_deref(), &text, suffix.as_deref())?,
            }),
            Request::Compact {
                session,
                retention,
                focus,
            } => Some(Arrival::Opened {
                session,
                round: Round::Compact,
                opened: self.door.compact(session, retention, focus)?,
            }),
            Request::Watch(session) => {
                self.watch(session)?;
                None
            }
            Request::Unwatch => {
                self.unwatch();
                None
            }
            Request::Bye => {
                self.bye();
                None
            }
        })
    }
}

fn kind_of(request: &Request) -> Kind {
    match request {
        Request::Call(_) => Kind::Read,
        Request::Post(_) => Kind::Change,
        Request::Submit { .. } | Request::Compact { .. } => Kind::Round,
        Request::Watch(_) | Request::Unwatch => Kind::Watch,
        Request::Bye => Kind::Change,
    }
}
