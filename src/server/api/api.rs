//! The api's public face: one connection table, one entry point per frame.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde_json::json;

use crate::ipc::contract::Frame;
use crate::server::agent::r#loop::register::Register;
use crate::server::storage::Writer;

use super::relay::Relay;
use super::route;

/// Why a door refused.
pub enum Refuse {
    Bad(String),
    Busy(String),
    Permission(String),
    Internal(String),
}

impl Refuse {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Refuse::Bad(_) => "bad_payload",
            Refuse::Busy(_) => "busy",
            Refuse::Permission(_) => "permission_denied",
            Refuse::Internal(_) => "internal",
        }
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Refuse::Bad(m) | Refuse::Busy(m) | Refuse::Permission(m) | Refuse::Internal(m) => m,
        }
    }
}

impl From<anyhow::Error> for Refuse {
    fn from(value: anyhow::Error) -> Self {
        Refuse::Internal(format!("{value:#}"))
    }
}

/// One connection's outbound end.
pub struct Conn {
    id: u64,
    tx: Sender<Frame>,
}

impl Conn {
    pub fn send(&self, frame: Frame) -> bool {
        self.tx.send(frame).is_ok()
    }
}

/// The backend's one surface for the ipc server.
pub struct Api {
    db: PathBuf,
    writer: Writer,
    register: Register,
    relay: Relay,
    next: AtomicU64,
}

impl Api {
    /// Open the writer, the relay's ports, and the loop's register, in that order.
    pub fn open(
        db: PathBuf,
        cfg: crate::server::config::Config,
        batch: usize,
    ) -> anyhow::Result<Api> {
        let writer = Writer::open(&db, batch)?;
        let (relay, events) = Relay::start(db.clone(), &writer)?;
        let register = Register::new(writer.clone(), cfg, events);
        Ok(Api::new(db, writer, register, relay))
    }

    pub fn new(db: PathBuf, writer: Writer, register: Register, relay: Relay) -> Api {
        Api {
            db,
            writer,
            register,
            relay,
            next: AtomicU64::new(1),
        }
    }

    /// Take one connection: its handle, and the queue its writer thread drains.
    pub fn connect(&self) -> (Conn, Receiver<Frame>) {
        let (tx, rx) = mpsc::channel();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.relay.attach(id, tx.clone());
        (Conn { id, tx }, rx)
    }

    /// One inbound frame: a reply to write back, or nothing.
    pub fn receive(&self, conn: &Conn, frame: Frame) -> Option<Frame> {
        match frame.kind.as_str() {
            "register" => self.register(conn, &frame),
            "call" | "post" | "subscribe" => {
                let Some(viewer) = self.relay.viewer_of(conn.id) else {
                    return Some(error(&frame, &Refuse::Permission("say hello first".into())));
                };
                if frame.kind == "subscribe" {
                    return match route::subscribe(self, conn.id, &viewer, &frame) {
                        Ok(None) => None,
                        Ok(Some(value)) => Some(Frame::reply_to(&frame, value)),
                        Err(refuse) => Some(error(&frame, &refuse)),
                    };
                }
                let answer = match frame.kind.as_str() {
                    "call" => route::call(self, &viewer, &frame),
                    _ => route::post(self, conn.id, &viewer, &frame),
                };
                Some(match answer {
                    Ok(value) => Frame::reply_to(&frame, value),
                    Err(refuse) => error(&frame, &refuse),
                })
            }
            _ => Some(error(
                &frame,
                &Refuse::Internal(format!("unknown kind: {}", frame.kind)),
            )),
        }
    }

    /// The connection is gone; its subscriptions go with it.
    pub fn disconnect(&self, conn: &Conn) {
        self.relay.detach(conn.id);
    }

    /// No connection for a whole grace period.
    pub fn idle_for(&self, grace: Duration) -> bool {
        if self.relay.live() > 0 {
            return false;
        }
        std::thread::sleep(grace);
        self.relay.live() == 0
    }

    pub(crate) fn db(&self) -> &Path {
        &self.db
    }

    pub(crate) fn writer(&self) -> &Writer {
        &self.writer
    }

    pub(crate) fn register(&self) -> &Register {
        &self.register
    }

    pub(crate) fn relay(&self) -> &Relay {
        &self.relay
    }

    fn register(&self, conn: &Conn, frame: &Frame) -> Option<Frame> {
        match frame.name.as_str() {
            "hello" => {
                let viewer = frame.payload.as_str().map(str::to_string).or_else(|| {
                    frame
                        .payload
                        .get("viewer")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                });
                let Some(viewer) = viewer else {
                    return Some(error(
                        frame,
                        &Refuse::Bad("hello carries no viewer id".into()),
                    ));
                };
                self.relay.hello(conn.id, viewer);
                Some(Frame::new("register", "opened", json!({})))
            }
            "bye" => {
                self.relay.detach(conn.id);
                None
            }
            _ => None,
        }
    }
}

fn error(request: &Frame, refuse: &Refuse) -> Frame {
    Frame::reply_to(
        request,
        json!({"error": {"code": refuse.code(), "message": refuse.message()}}),
    )
}
