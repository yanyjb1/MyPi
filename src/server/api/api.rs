//! The api's public face: one connection table, one entry point per frame.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde_json::{Value, json};

use crate::ipc::contract::ErrorCode;
use crate::ipc::contract::Frame;
use crate::server::agent::r#loop::{Birth, Loop, Settings};
use crate::server::agent::request::message::ToolDef;
use crate::server::agent::request::{Options, Request, RetryPolicy, Streaming};
use crate::server::config::Config;
use crate::server::storage::{Reader, Writer};

use super::relay::Relay;
use super::route;

/// Why a door refused.
pub enum Refuse {
    Unknown(String),
    Bad(String),
    Busy(String),
    Permission(String),
    Internal(String),
}

impl Refuse {
    pub(crate) fn code(&self) -> ErrorCode {
        match self {
            Refuse::Unknown(_) => ErrorCode::UnknownName,
            Refuse::Bad(_) => ErrorCode::BadPayload,
            Refuse::Busy(_) => ErrorCode::Busy,
            Refuse::Permission(_) => ErrorCode::PermissionDenied,
            Refuse::Internal(_) => ErrorCode::Internal,
        }
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Refuse::Unknown(m)
            | Refuse::Bad(m)
            | Refuse::Busy(m)
            | Refuse::Permission(m)
            | Refuse::Internal(m) => m,
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
    cfg: Config,
    ledger: Loop,
    relay: Relay,
    /// What the configuration had to say at load: handed to every front end as it opens.
    notices: Value,
    next: AtomicU64,
}

impl Api {
    /// Open the writer, the relay, the request half and the loop, in that order.
    pub fn open(db: PathBuf, cfg: Config) -> anyhow::Result<Api> {
        let writer = Writer::open(&db)?;
        let (relay, events) = Relay::start()?;
        let settings = Settings {
            max_tool_rounds: cfg.tool_rounds(),
            scrooge: cfg.scrooge(),
        };
        let options = Options {
            providers: cfg.providers(),
            retry: RetryPolicy {
                max_retries: cfg.retry_attempts(),
                interval: cfg.retry_interval(),
                timeout: cfg.retry_timeout(),
            },
            output_caps: cfg.output_caps(),
            streaming: if cfg.streaming_immediate() {
                Streaming::Immediate
            } else {
                Streaming::Buffered
            },
            retain_tail: cfg.retain_tail(),
        };
        let ask = Arc::new(Request::new(options)?);
        let ledger = Loop::new(writer.clone(), settings, ask, events, &cfg);
        // A session nobody watches any more is the loop's to unbook.
        relay.close_with({
            let ledger = ledger.clone();
            Arc::new(move |session: i64| ledger.drop(session))
        });
        let notices = startup_notices(&cfg);
        Ok(Api {
            db,
            writer,
            cfg,
            ledger,
            relay,
            notices,
            next: AtomicU64::new(1),
        })
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
                let viewer = match self.relay.viewer_of(conn.id) {
                    Some(viewer) => viewer,
                    None => {
                        return Some(error(&frame, &Refuse::Permission("say hello first".into())));
                    }
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
                &Refuse::Unknown(format!("unknown kind: {}", frame.kind)),
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

    /// Put a session the loop does not know back on the table, out of its own rows.
    pub(crate) fn ensure_booked(&self, session: i64) -> Result<(), Refuse> {
        if self.ledger.booked(session) {
            return Ok(());
        }
        let reader = Reader::open(&self.db)?;
        let row = reader
            .session(session)?
            .ok_or_else(|| Refuse::Bad(format!("no session {session}")))?;
        let meta = row["meta"].as_i64();
        let archive = match meta {
            Some(id) => reader.request(id)?,
            None => None,
        };
        let archive = match archive {
            Some(archive) => archive,
            None => {
                return Err(Refuse::Internal(
                    "the session has no request archive".into(),
                ));
            }
        };
        let system = archive["system"].as_str().unwrap_or_default().to_string();
        let manuals: Vec<ToolDef> = serde_json::from_value(archive["tools"].clone())
            .map_err(|e| Refuse::Internal(format!("the archive's tools are not manuals: {e}")))?;
        let manuals = Arc::new(manuals);
        let tools: Vec<String> = manuals
            .iter()
            .map(|manual| manual.function.name.clone())
            .collect();
        let stated = match (row["provider"].as_str(), row["model"].as_str()) {
            (Some(provider), Some(model)) => Some((provider.to_string(), model.to_string())),
            _ => None,
        };
        let cwd = row["cwd"]
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(|| Refuse::Internal("the session row carries no cwd".into()))?;
        drop(reader);

        let cfg = self.cfg();
        let profile = cfg.resolve_profile(None, &cwd)?;
        let (provider, model) = match stated {
            Some(pair) => pair,
            None => {
                let default = cfg
                    .default_model(None, &cwd)
                    .ok_or_else(|| Refuse::Internal("no default model is configured".into()))?;
                (default.provider_name, default.entry.id)
            }
        };

        self.ledger.book(Birth {
            session,
            provider,
            model,
            system,
            tools,
            manuals,
            compact: profile.compact_instruction,
            meta,
            cwd,
        })?;
        Ok(())
    }

    pub(crate) fn db(&self) -> &Path {
        &self.db
    }

    pub(crate) fn writer(&self) -> &Writer {
        &self.writer
    }

    pub(crate) fn cfg(&self) -> &Config {
        &self.cfg
    }

    pub(crate) fn ledger(&self) -> &Loop {
        &self.ledger
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
                let viewer = match viewer {
                    Some(viewer) => viewer,
                    None => {
                        return Some(error(
                            frame,
                            &Refuse::Bad("hello carries no viewer id".into()),
                        ));
                    }
                };
                self.relay.hello(conn.id, viewer);
                Some(Frame::new(
                    "register",
                    "opened",
                    json!({ "notices": self.notices }),
                ))
            }
            "bye" => {
                self.relay.detach(conn.id);
                None
            }
            _ => None,
        }
    }
}

/// What the configuration said at load, as a front end reads it: one line per notice.
fn startup_notices(cfg: &Config) -> Value {
    let lines: Vec<Value> = cfg
        .notices(None)
        .iter()
        .map(|notice| {
            json!({
                "level": "warn",
                "text": format!("{}：{}（本次使用 {}）", notice.file, notice.problem, notice.used),
            })
        })
        .collect();
    Value::Array(lines)
}

fn error(request: &Frame, refuse: &Refuse) -> Frame {
    Frame::error_to(request, refuse.code(), refuse.message())
}
