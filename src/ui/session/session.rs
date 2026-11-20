// The session: one connection, two windows, one outbox; every frame crosses unchanged.

use anyhow::Result;
use serde_json::{Value, json};

use crate::ipc::contract::Frame;
use crate::ipc::endpoint::Endpoint;

use super::client::{Client, Incoming};
use super::content::ContentCache;
use super::packet::Packet;
use super::structure::{Structure, Turn, ranges_of, turns_of};

// The configuration key that sizes the content window.
const BUDGET_KEY: &str = "config.ui.session.preloadCache";

pub struct Session {
    client: Client,
    // The session this connection follows.
    attached: Option<i64>,
    structure: Structure,
    content: ContentCache,
    // What arrived since the last `forward`, in arrival order.
    outbox: Vec<Packet>,
    // The connection died once; the front end is told once.
    shut: bool,
}

impl Session {
    // Connect and register. Who starts the server is not this layer's business.
    pub fn connect(endpoint: Endpoint) -> Result<Session> {
        Ok(Session {
            client: Client::connect(endpoint)?,
            attached: None,
            structure: Structure::default(),
            content: ContentCache::default(),
            outbox: Vec::new(),
            shut: false,
        })
    }

    // Hand the front end everything the connection produced since the last call.
    pub fn forward(&mut self) -> Vec<Packet> {
        for item in self.client.pump() {
            self.ingest(item);
        }
        if !self.shut && !self.client.alive() {
            self.shut = true;
            self.outbox.push(Packet::local("disconnected", json!({})));
        }
        std::mem::take(&mut self.outbox)
    }

    // Follow one session. The caller subscribes the watch; the windows go now and the budget is asked again.
    pub fn attach(&mut self, id: i64) {
        self.attached = Some(id);
        self.client.clear();
        self.structure.clear();
        self.content.clear();
        self.config(BUDGET_KEY);
    }

    // Stop following on purpose: whatever the old session still owed goes, then the slot and watch, then the windows.
    pub fn detach(&mut self) {
        let id = self.attached;
        self.client.clear();
        if let Some(id) = id {
            self.clear_remote_mailbox(id);
        }
        self.unwatch();
        // Both frames have to leave before `forget` drops what is still unsent.
        let _ = self.client.pump();
        self.forget();
    }

    // Stop following without a word: the server already closed the session.
    fn forget(&mut self) {
        self.attached = None;
        self.client.clear();
        self.structure.clear();
        self.content.clear();
    }

    pub fn attached(&self) -> Option<i64> {
        self.attached
    }

    // The structural window as it stands: the one snapshot this layer hands down.
    pub fn structure(&self) -> Structure {
        self.structure.clone()
    }

    // Queue one frame. `answer` says whether the server replies to it.
    pub fn send(&mut self, kind: &str, name: &str, content: Value, answer: bool) {
        self.client.queue(Frame::new(kind, name, content), answer);
    }

    // Queue one action frame: this session's id goes in, a reply is owed.
    pub(super) fn act(&mut self, kind: &str, name: &str, content: Value) {
        let content = self.with_session(content);
        self.send(kind, name, content, true);
    }

    // Queue one meta write: the door rides inside the frame, its arguments in `content`.
    pub(super) fn meta_write(&mut self, name: &str, content: Value) {
        let content = self.with_session(content);
        self.send(
            "post",
            "meta",
            json!({"name": name, "content": content}),
            true,
        );
    }

    fn with_session(&self, content: Value) -> Value {
        match (self.attached, content) {
            (Some(id), Value::Object(mut map)) => {
                map.entry("session").or_insert(json!(id));
                Value::Object(map)
            }
            (_, content) => content,
        }
    }

    // Say goodbye and let the connection go: everything queued leaves, then `bye`.
    pub fn bye(&mut self) {
        self.sign_off();
        let _ = self.client.pump();
    }

    // A block read: answered from the window when it covers the ranges.
    pub(super) fn read_blocks(&mut self, ranges: Vec<[i64; 2]>) {
        let session = self
            .attached
            .expect("a block read happens inside one attachment");
        if self.content.covers(&ranges) {
            let blocks = self.content.select(&ranges);
            self.outbox
                .push(Packet::new("call", "blocks", json!({"blocks": blocks})));
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

    fn ingest(&mut self, item: Incoming) {
        match item {
            Incoming::Push(frame) => self.push(frame),
            Incoming::Answer { request, reply } => self.answer(request, reply),
        }
    }

    // One unsolicited frame: keep the windows honest for the attached session, forward it whole.
    fn push(&mut self, frame: Frame) {
        let content = frame.payload;
        let foreign = content
            .get("session")
            .and_then(Value::as_i64)
            .is_some_and(|session| self.attached != Some(session));
        if !foreign
            && let Some(turn) = content.get("turn").and_then(Value::as_i64)
            && let Some(ranges) = content.get("ranges")
        {
            let compact = content
                .get("compact")
                .and_then(Value::as_bool)
                .expect("a structure frame marks its compaction");
            self.structure
                .set(Turn::new(turn, ranges_of(ranges), compact));
        }
        self.outbox
            .push(Packet::new(frame.kind, frame.name, content));
    }

    // One answer: keep the windows honest, forward it whole.
    fn answer(&mut self, request: Frame, reply: Frame) {
        match (request.kind.as_str(), request.name.as_str()) {
            ("call", "init") => {
                if !refused(&reply) {
                    self.structure.replace(turns_of(
                        reply
                            .payload
                            .get("structure")
                            .expect("call init answers its structure"),
                    ));
                    let blocks = reply
                        .payload
                        .get("blocks")
                        .and_then(Value::as_array)
                        .expect("call init answers its blocks");
                    self.content.replace(blocks.iter().cloned());
                }
            }
            ("call", "blocks") => {
                if !refused(&reply) {
                    let blocks = reply
                        .payload
                        .get("blocks")
                        .and_then(Value::as_array)
                        .expect("call blocks answers its blocks");
                    self.content.merge(blocks.iter().cloned());
                }
            }
            ("call", "config") => {
                let key = request
                    .payload
                    .get("key")
                    .and_then(Value::as_str)
                    .expect("call config carries a key");
                if key == BUDGET_KEY {
                    let budget = reply
                        .payload
                        .as_u64()
                        .expect("preloadCache answers a count");
                    self.content.set_budget(budget as usize);
                    return;
                }
                self.outbox.push(Packet::new(
                    "call",
                    "config",
                    json!({"key": key, "value": reply.payload}),
                ));
                return;
            }
            ("post", "meta") => self.rewind_landed(&request, &reply),
            ("post", "delete") => {
                let session = request
                    .payload
                    .get("session")
                    .and_then(Value::as_i64)
                    .expect("post delete names its session");
                if !refused(&reply) && Some(session) == self.attached {
                    self.forget();
                }
            }
            _ => {}
        }
        self.outbox
            .push(Packet::new(reply.kind, reply.name, reply.payload));
    }

    // A `post meta` answer may be this layer's own rewind; the spot comes from the request.
    fn rewind_landed(&mut self, request: &Frame, reply: &Frame) {
        let name = request
            .payload
            .get("name")
            .and_then(Value::as_str)
            .expect("post meta names its door");
        if name != "rewind" || refused(reply) {
            return;
        }
        let content = request
            .payload
            .get("content")
            .expect("post meta carries its arguments");
        let turn = content
            .get("turn")
            .and_then(Value::as_i64)
            .expect("rewind names its turn");
        let block = content
            .get("block")
            .and_then(Value::as_i64)
            .expect("rewind names its block");
        self.structure.cut_at(turn, block);
        self.content.retain(|id| self.structure.owns(id));
    }
}

// Whether the server refused a write.
fn refused(reply: &Frame) -> bool {
    reply.payload.get("error").is_some()
}
