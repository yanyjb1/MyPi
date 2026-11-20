// The session: one connection, two windows, one outbox.
// Every frame crosses unchanged; only the few that keep its windows are read.

use anyhow::Result;
use serde_json::{Value, json};

use crate::ipc::contract::Frame;
use crate::ipc::endpoint::Endpoint;

use super::client::{Client, Incoming};
use super::content::ContentCache;
use super::packet::Packet;
use super::structure::{Structure, Turn};

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
            client: Client::connect(endpoint, viewer())?,
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
            self.take(item);
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
        let content = self.filled(content);
        self.send(kind, name, content, true);
    }

    // Queue one meta write: the door rides inside the frame, its arguments in `content`.
    pub(super) fn meta_write(&mut self, name: &str, content: Value) {
        let content = self.filled(content);
        self.send(
            "post",
            "meta",
            json!({"name": name, "content": content}),
            true,
        );
    }

    fn filled(&self, content: Value) -> Value {
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
    pub(super) fn read_blocks(&mut self, content: Value) {
        let session = content
            .get("session")
            .and_then(Value::as_i64)
            .or(self.attached);
        let ranges = content.get("ranges").and_then(ranges_of);
        let (Some(session), Some(ranges)) = (session, ranges) else {
            self.client
                .queue(Frame::new("call", "blocks", content), true);
            return;
        };
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

    fn take(&mut self, item: Incoming) {
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
            && let Some(ranges) = content.get("ranges").and_then(ranges_of)
        {
            let compact = content
                .get("compact")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            self.structure.set(Turn::new(turn, ranges, compact));
        }
        self.outbox
            .push(Packet::new(frame.kind, frame.name, content));
    }

    // One answer: keep the windows honest, forward it whole.
    fn answer(&mut self, request: Frame, reply: Frame) {
        match (request.kind.as_str(), request.name.as_str()) {
            ("call", "init") => {
                if let Some(turns) = reply.payload.get("structure").and_then(turns_of) {
                    self.structure.replace(turns);
                }
                if let Some(blocks) = reply.payload.get("blocks").and_then(Value::as_array) {
                    self.content.replace(blocks.iter().cloned());
                }
            }
            ("call", "blocks") => {
                if let Some(blocks) = reply.payload.get("blocks").and_then(Value::as_array) {
                    self.content.merge(blocks.iter().cloned());
                }
            }
            ("call", "config") => {
                let key = request.payload.get("key").and_then(Value::as_str);
                if key == Some(BUDGET_KEY) {
                    if let Some(budget) = reply.payload.as_u64() {
                        self.content.set_budget(budget as usize);
                    }
                    return;
                }
                if let Some(key) = key {
                    self.outbox.push(Packet::new(
                        "call",
                        "config",
                        json!({"key": key, "value": reply.payload}),
                    ));
                    return;
                }
            }
            ("post", "meta") => self.rewind_landed(&request, &reply),
            ("post", "delete") => {
                let session = request.payload.get("session").and_then(Value::as_i64);
                if !refused(&reply) && session == self.attached {
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
        if refused(reply) {
            return;
        }
        if request.payload.get("name").and_then(Value::as_str) != Some("rewind") {
            return;
        }
        let Some(content) = request.payload.get("content") else {
            return;
        };
        let (Some(turn), Some(block)) = (
            content.get("turn").and_then(Value::as_i64),
            content.get("block").and_then(Value::as_i64),
        ) else {
            return;
        };
        self.structure.cut_at(turn, block);
        self.content.retain(|id| self.structure.owns(id));
    }
}

// Whether the server refused a write.
fn refused(reply: &Frame) -> bool {
    reply.payload.get("error").is_some()
}

// A frame's ranges: `[[lo, hi], …]`.
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

// An `init` answer's `structure`: one `{turn, ranges, compact}` per round.
fn turns_of(value: &Value) -> Option<Vec<Turn>> {
    let mut out = Vec::new();
    for entry in value.as_array()? {
        out.push(Turn::new(
            entry.get("turn")?.as_i64()?,
            ranges_of(entry.get("ranges")?)?,
            entry
                .get("compact")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ));
    }
    out.reverse();
    Some(out)
}

// One identity for this front end instance.
fn viewer() -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Value::String(format!("{}-{now}", std::process::id()))
}
