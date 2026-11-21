// One session: every frame forwarded unchanged, and the structural window kept in step.

use anyhow::Result;
use serde_json::{Value, json};

use crate::ipc::contract::Frame;
use crate::ipc::endpoint::Endpoint;

use super::down::Down;
use super::structure::{Structure, ranges_of};
use super::up::Up;

pub struct Session {
    // The conversation this connection follows; `None` until a row exists.
    attached: Option<i64>,
    up: Up,
    down: Down,
    // Set once the read half dies, so the front end hears it once.
    shut: bool,
}

impl Session {
    pub fn open(endpoint: Endpoint, id: Option<i64>) -> Result<Session> {
        Ok(Session {
            attached: id,
            up: Up::connect(endpoint)?,
            down: Down::default(),
            shut: false,
        })
    }

    // Follow one conversation; the window the last one showed goes with it.
    pub fn attach(&mut self, id: i64) {
        self.attached = Some(id);
        self.down.structure_mut().clear();
    }

    pub fn detach(&mut self) {
        self.attached = None;
        self.down.structure_mut().clear();
    }

    // The slot is filled when the writer left it out: a zone reading another conversation keeps its own.
    pub fn send(&self, mut frame: Frame) {
        if let (Some(id), Some(object)) = (self.attached, frame.payload.as_object_mut())
            && !object.contains_key("session")
        {
            object.insert("session".to_string(), json!(id));
        }
        self.up.send(frame);
    }

    pub fn receive(&mut self) -> Vec<Frame> {
        for frame in self.up.drain() {
            self.absorb(&frame.payload);
            self.down.push(frame);
        }
        if !self.shut && !self.up.alive() {
            self.shut = true;
            self.down
                .push(Frame::new("local", "disconnected", json!({})));
        }
        self.down.receive()
    }

    pub fn structure(&self) -> &Structure {
        self.down.structure()
    }

    pub fn close(&self) {
        self.up.send(Frame::new("register", "bye", json!({})));
    }

    // Only the attached conversation's structure reaches the window: a page may read another.
    fn absorb(&mut self, content: &Value) {
        if !self.mine(content) {
            return;
        }
        if let Some(signal) = content.get("signal").and_then(Value::as_str) {
            if signal == "rewind"
                && let Some(block) = content.get("block").and_then(Value::as_i64)
            {
                self.down
                    .structure_mut()
                    .update(None, &[[block, block]], false);
            }
            return;
        }
        if let Some(snapshot) = content.get("structure").filter(|value| value.is_array()) {
            *self.down.structure_mut() = Structure::of(snapshot);
            return;
        }
        self.round(content);
    }

    // Whether a frame speaks about the conversation this connection follows.
    fn mine(&self, content: &Value) -> bool {
        match (
            content.get("session").and_then(Value::as_i64),
            self.attached,
        ) {
            (Some(said), Some(attached)) => said == attached,
            (Some(_), None) => false,
            (None, _) => true,
        }
    }

    fn round(&mut self, value: &Value) {
        let Some(turn) = value.get("turn").and_then(Value::as_i64) else {
            return;
        };
        let Some(ranges) = value.get("ranges").and_then(ranges_of) else {
            return;
        };
        let compact = value
            .get("compact")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.down
            .structure_mut()
            .update(Some(turn), &ranges, compact);
    }
}
