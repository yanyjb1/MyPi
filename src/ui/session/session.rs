//! The session: one connection, two windows, one outbox.
//!
//! Every frame crosses unchanged. The only things read out of a frame are the ones the windows
//! cannot be kept without: a push's session and turn, a round's ranges, a row's id, the spot a
//! rewind was asked for.

use std::collections::VecDeque;

use anyhow::Result;
use serde_json::{Value, json};

use crate::ipc::contract::Frame;
use crate::ipc::endpoint::Endpoint;

use super::client::{Client, Incoming};
use super::content::ContentCache;
use super::packet::Packet;
use super::structure::{Structure, Turn};

/// The configuration key that sizes the content window.
const BUDGET_KEY: &str = "config.ui.session.preloadCache";

/// A change this front end asked for and has not seen land yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    /// A rewind: the structure goes back to this spot once the write is accepted.
    Rewind { turn: i64, block: i64 },
}

pub struct Session {
    client: Client,
    /// The session this connection follows; pushes for another are dropped.
    attached: Option<i64>,
    structure: Structure,
    content: ContentCache,
    /// What was asked for and not yet answered, oldest first.
    pending: VecDeque<Pending>,
    /// What arrived since the last `forward`, in arrival order.
    outbox: Vec<Packet>,
    /// The connection died once; the front end is told once.
    shut: bool,
}

impl Session {
    /// Connect and register. Who starts the server is not this layer's business.
    pub fn connect(endpoint: Endpoint) -> Result<Session> {
        let client = Client::connect(endpoint, viewer())?;
        let mut session = Session {
            client,
            attached: None,
            structure: Structure::default(),
            content: ContentCache::default(),
            pending: VecDeque::new(),
            outbox: Vec::new(),
            shut: false,
        };
        session.client.queue(
            Frame::new("call", "config", json!({"key": BUDGET_KEY})),
            true,
        );
        Ok(session)
    }

    /// Hand the front end everything the connection produced since the last call.
    pub fn forward(&mut self) -> Vec<Packet> {
        for item in self.client.pump() {
            self.take(item);
        }
        if !self.shut && !self.client.alive() {
            self.shut = true;
            self.outbox.push(Packet::local("disconnected", json!({})));
        }
        std::mem::take(&mut self.outbox)
    }

    /// Follow one session. The caller subscribes the watch; the windows go now.
    pub fn attach(&mut self, id: i64) {
        self.attached = Some(id);
        self.structure.clear();
        self.content.clear();
        self.pending.clear();
    }

    /// Stop following. The watch is replaced by the next one.
    pub fn detach(&mut self) {
        if let Some(id) = self.attached {
            // The slot this front end wrote is cleared before the queue goes; no answer is owed.
            self.client.clear();
            self.client
                .queue(Frame::new("post", "mailbox", json!({"session": id})), false);
            let _ = self.client.pump();
        }
        self.attached = None;
        self.client.clear();
        self.unwatch();
        self.structure.clear();
        self.content.clear();
        self.pending.clear();
    }

    pub fn attached(&self) -> Option<i64> {
        self.attached
    }

    /// The structural window as it stands: the one snapshot this layer hands down.
    pub fn structure(&self) -> Structure {
        self.structure.clone()
    }

    /// Queue one frame. `answer` says whether the server replies to it.
    pub fn send(&mut self, kind: &str, name: &str, content: Value, answer: bool) {
        if kind == "call" && name == "blocks" {
            self.read_blocks(content);
            return;
        }
        // A rewind is owed a trim of this layer's own index; the spot comes from the
        // request, never from the reply, so it is noted before the frame leaves.
        if kind == "post"
            && name == "rewind"
            && let (Some(turn), Some(block)) = (
                content.get("turn").and_then(Value::as_i64),
                content.get("block").and_then(Value::as_i64),
            )
        {
            self.pending.push_back(Pending::Rewind { turn, block });
        }
        self.client.queue(Frame::new(kind, name, content), answer);
    }

    /// Queue one action frame: this session's id goes in, a reply is owed.
    pub(super) fn act(&mut self, kind: &str, name: &str, content: Value) {
        let content = match (self.attached, content) {
            (Some(id), Value::Object(mut map)) => {
                map.entry("session").or_insert(json!(id));
                Value::Object(map)
            }
            (_, content) => content,
        };
        self.send(kind, name, content, true);
    }

    /// Say goodbye and let the connection go: everything queued leaves, then `bye`.
    pub fn bye(&mut self) {
        self.client
            .queue(Frame::new("register", "bye", json!({})), false);
        let _ = self.client.pump();
    }

    /// A block read: answered from the window when it covers the ranges.
    fn read_blocks(&mut self, content: Value) {
        let session = content
            .get("session")
            .and_then(Value::as_i64)
            .or(self.attached);
        let Some(session) = session else {
            return;
        };
        let Some(ranges) = content.get("ranges").and_then(ranges_of) else {
            return;
        };
        if self.content.covers(&ranges) {
            let rows = self.content.select(&ranges);
            self.outbox
                .push(Packet::new("call", "blocks", json!({"blocks": rows})));
            return;
        }
        self.client.queue(
            Frame::new(
                "call",
                "blocks",
                json!({"session": session, "ranges": ranges}),
            ),
            true,
        );
    }

    fn take(&mut self, item: Incoming) {
        match item {
            Incoming::Push(frame) => self.push(frame),
            Incoming::Answer { request, reply } => self.answer(request, reply),
            Incoming::Lost { request, reason } => {
                self.outbox.push(Packet::local(
                    "lost",
                    json!({
                        "kind": request.kind,
                        "name": request.name,
                        "reason": reason,
                    }),
                ));
            }
        }
    }

    /// One unsolicited frame: keep the windows honest, forward it whole.
    fn push(&mut self, frame: Frame) {
        let content = frame.payload;
        let session = content.get("session").and_then(Value::as_i64);
        if session != self.attached {
            return;
        }
        if let Some(turn) = content.get("turn").and_then(Value::as_i64)
            && let Some(ranges) = content.get("ranges").and_then(ranges_of)
        {
            self.structure.set(Turn::new(turn, ranges));
        }
        self.outbox
            .push(Packet::new(frame.kind, frame.name, content));
    }

    /// One answer: keep the windows honest, forward it whole.
    fn answer(&mut self, request: Frame, reply: Frame) {
        match (request.kind.as_str(), request.name.as_str()) {
            ("call", "init") => {
                if let Some(turns) = reply.payload.get("structure").and_then(turns_of) {
                    self.structure.replace(turns);
                }
                if let Some(rows) = reply.payload.get("blocks").and_then(Value::as_array) {
                    self.content.replace(rows.iter().cloned());
                }
            }
            ("call", "blocks") => {
                if let Some(rows) = reply.payload.get("blocks").and_then(Value::as_array) {
                    self.content.merge(rows.iter().cloned());
                }
            }
            ("call", "config") => {
                let Some(key) = request.payload.get("key").and_then(Value::as_str) else {
                    return;
                };
                if key == BUDGET_KEY {
                    if let Some(budget) = reply.payload.as_i64() {
                        self.content.set_budget(budget.max(1) as usize);
                    }
                    // This layer's own question is not the front end's mail.
                    return;
                }
                // The answer names the key it answers: three readers share one bus.
                self.outbox.push(Packet::new(
                    "call",
                    "config",
                    json!({"key": key, "value": reply.payload}),
                ));
                return;
            }
            ("post", "rewind") => {
                // The ledger is the whole of this layer's interest: an accepted write is
                // one that goes through, and the spot comes from the request.
                match self.pending.pop_front() {
                    Some(Pending::Rewind { turn, block })
                        if reply.payload.get("error").is_none() =>
                    {
                        self.structure.cut_at(turn, block);
                        self.content.retain(|id| self.structure.owns(id));
                    }
                    _ => {}
                }
            }
            ("post", "delete") => {
                let session = request.payload.get("session").and_then(Value::as_i64);
                if session.is_some() && session == self.attached {
                    self.detach();
                }
            }
            _ => {}
        }
        self.outbox
            .push(Packet::new(reply.kind, reply.name, reply.payload));
    }
}

/// A frame's ranges: `[[lo, hi], …]`.
fn ranges_of(value: &Value) -> Option<Vec<[i64; 2]>> {
    let mut out = Vec::new();
    for item in value.as_array()? {
        let pair = item.as_array()?;
        if pair.len() != 2 {
            return None;
        }
        out.push([pair[0].as_i64()?, pair[1].as_i64()?]);
    }
    Some(out)
}

/// An `init` answer's `structure`: one `{turn, ranges}` per round.
fn turns_of(value: &Value) -> Option<Vec<Turn>> {
    let mut out = Vec::new();
    for row in value.as_array()? {
        out.push(Turn::new(
            row.get("turn")?.as_i64()?,
            ranges_of(row.get("ranges")?)?,
        ));
    }
    out.reverse();
    Some(out)
}

/// One identity for this front end instance.
fn viewer() -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Value::String(format!("{}-{now}", std::process::id()))
}
