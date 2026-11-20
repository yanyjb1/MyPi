// The api's public face: one connection table, one entry point per frame.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde_json::{Value, json};

use crate::ipc::contract::Frame;
use crate::ipc::contract::Text;
use crate::server::agent::r#loop::{Birth, Loop, Settings};
use crate::server::agent::request::message::ToolDef;
use crate::server::agent::request::{Options, Request, RetryPolicy, Streaming};
use crate::server::config::Config;
use crate::server::storage::{Reader, Writer};

use super::relay::Relay;
use super::route;
use super::subscribe;

// Why a write refused: a machine key for the message, plus optional template arguments.
pub struct Refuse(pub Text);

impl Refuse {
    pub fn new(key: impl Into<String>) -> Self {
        Refuse(Text::error(key))
    }

    pub fn args(self, args: Value) -> Self {
        Refuse(self.0.args(args))
    }
}

impl From<anyhow::Error> for Refuse {
    fn from(_: anyhow::Error) -> Self {
        Refuse::new("internal")
    }
}

// One connection's outbound end.
pub struct Conn {
    id: u64,
    tx: Sender<Frame>,
}

impl Conn {
    pub fn send(&self, frame: Frame) -> bool {
        self.tx.send(frame).is_ok()
    }
}

// The backend's one surface for the ipc server.
pub struct Api {
    db: PathBuf,
    writer: Writer,
    cfg: Config,
    ledger: Loop,
    relay: Relay,
    // What the configuration had to say at load: handed to every front end as it opens.
    notices: Value,
    next: AtomicU64,
}

impl Api {
    // Open the writer, the relay, the request half and the loop, in that order.
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
        let ledger = Loop::new(writer.clone(), settings, ask, events, &cfg)?;
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

    // Take one connection: its handle, and the queue its writer thread drains.
    pub fn connect(&self) -> (Conn, Receiver<Frame>) {
        let (tx, rx) = mpsc::channel();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.relay.attach(id, tx.clone());
        (Conn { id, tx }, rx)
    }

    // One inbound frame: a reply to write back, or nothing.
    pub fn receive(&self, conn: &Conn, frame: Frame) -> Option<Frame> {
        match frame.kind.as_str() {
            "register" => self.register(conn, &frame),
            "call" | "post" | "subscribe" => {
                let viewer = match self.relay.viewer_of(conn.id) {
                    Some(viewer) => viewer,
                    None => {
                        return Some(error(&frame, &Refuse::new("hello_required")));
                    }
                };
                if frame.kind == "subscribe" {
                    return match route::subscribe(self, conn.id, &viewer, &frame) {
                        Ok(None) => None,
                        Ok(Some(value)) => Some(Frame::reply_to(&frame, value)),
                        Err(refuse) => Some(error(&frame, &refuse)),
                    };
                }
                if frame.kind == "post" {
                    return match route::post(self, conn.id, &viewer, &frame) {
                        Ok(reply) => {
                            if let Some(signal) = reply.signal {
                                self.relay.signal(signal, &viewer);
                            }
                            for session in &reply.closed {
                                subscribe::close::apply(self, *session);
                            }
                            Some(Frame::reply_to(&frame, reply.done))
                        }
                        Err(refuse) => Some(error(&frame, &refuse)),
                    };
                }
                Some(match route::call(self, &viewer, &frame) {
                    Ok(value) => Frame::reply_to(&frame, value),
                    Err(refuse) => error(&frame, &refuse),
                })
            }
            _ => Some(error(
                &frame,
                &Refuse::new("unknown_kind").args(json!({"kind": frame.kind.clone()})),
            )),
        }
    }

    // The connection is gone; its subscriptions go with it.
    pub fn disconnect(&self, conn: &Conn) {
        self.relay.detach(conn.id);
    }

    // Front ends connected right now.
    pub(crate) fn live(&self) -> usize {
        self.relay.live()
    }

    // No connection for a whole grace period.
    pub fn idle_for(&self, grace: Duration) -> bool {
        if self.relay.live() > 0 {
            return false;
        }
        std::thread::sleep(grace);
        self.relay.live() == 0
    }

    // Put a session the loop does not know back on the table, out of its own rows.
    pub(crate) fn ensure_booked(&self, session: i64) -> Result<(), Refuse> {
        if self.ledger.booked(session) {
            return Ok(());
        }
        let reader = Reader::open(&self.db)?;
        let row = reader
            .session(session)?
            .ok_or_else(|| Refuse::new("no_session").args(json!({"session": session})))?;
        let meta = row["meta"].as_i64();
        let archive = match meta {
            Some(id) => reader.request(id)?,
            None => None,
        };
        let archive = match archive {
            Some(archive) => archive,
            None => {
                return Err(Refuse::new("no_archive"));
            }
        };
        let system = archive["system"].as_str().unwrap_or_default().to_string();
        let manuals: Vec<ToolDef> = serde_json::from_value(archive["tools"].clone())
            .map_err(|_| Refuse::new("internal"))?;
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
            .ok_or_else(|| Refuse::new("no_cwd"))?;
        drop(reader);

        let cfg = self.cfg();
        let profile = cfg.resolve_profile(None, &cwd)?;
        let (provider, model) = match stated {
            Some(pair) => pair,
            None => {
                let default = cfg
                    .default_model(None, &cwd)
                    .ok_or_else(|| Refuse::new("no_default_model"))?;
                (default.provider_name, default.entry.id)
            }
        };

        let vision = cfg
            .model_by_id(&format!("{provider}:{model}"))
            .map(|resolved| resolved.entry.vision)
            .unwrap_or(false);
        self.ledger.book(Birth {
            session,
            provider,
            model,
            system,
            tools,
            manuals,
            compact: profile.compact_instruction,
            effort: crate::server::agent::request::Effort::Auto,
            vision,
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
                        return Some(error(frame, &Refuse::new("hello_no_viewer")));
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

// What the configuration said at load, as a front end reads it: one line per notice.
fn startup_notices(cfg: &Config) -> Value {
    let notices: Vec<Value> = cfg
        .notices(None)
        .iter()
        .map(|notice| {
            let mut args = notice.args.clone().unwrap_or_else(|| json!({}));
            if let Some(object) = args.as_object_mut() {
                object.insert("file".to_string(), json!(notice.file.clone()));
                object.insert("used".to_string(), json!(notice.used.clone()));
            }
            json!({"kind": "warning", "key": notice.problem, "args": args})
        })
        .collect();
    Value::Array(notices)
}

fn error(request: &Frame, refuse: &Refuse) -> Frame {
    Frame::error_to(request, refuse.0.clone())
}
