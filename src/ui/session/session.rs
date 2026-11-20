// One session: every frame forwarded unchanged, and the structural window kept in step.

use anyhow::Result;
use serde_json::{Value, json};

use crate::ipc::contract::Frame;
use crate::ipc::endpoint::Endpoint;

use super::down::Down;
use super::structure::Structure;
use super::up::Up;

pub struct Session {
    id: i64,
    up: Up,
    down: Down,
    // Set once the read half dies, so the front end hears it once.
    shut: bool,
}

impl Session {
    pub fn open(endpoint: Endpoint, id: i64) -> Result<Session> {
        Ok(Session {
            id,
            up: Up::connect(endpoint)?,
            down: Down::default(),
            shut: false,
        })
    }

    pub fn id(&self) -> i64 {
        self.id
    }

    pub fn send(&self, mut frame: Frame) {
        if let Some(object) = frame.payload.as_object_mut() {
            object.insert("session".to_string(), json!(self.id));
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

    fn absorb(&mut self, content: &Value) {
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
        if let Some(rounds) = content.get("structure").and_then(Value::as_array) {
            for round in rounds {
                self.round(round);
            }
            return;
        }
        self.round(content);
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

// A round's ranges: `[[lo, hi], …]`.
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
