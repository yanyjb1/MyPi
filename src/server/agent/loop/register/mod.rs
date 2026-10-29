// The registry: one slot per session, the only writer of every Submit.

mod build;
mod compact;
mod outlets;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, Weak};

use anyhow::{Context as _, Result};

use crate::server::agent::r#loop::outlet::{Event, EventOutlet};
use crate::server::agent::r#loop::{self as agent_loop, Live, Turn};
use crate::server::agent::request::assembly::compact as cut;

use crate::server::agent::tools::McpBridge;
use crate::server::agent::tools::jobs::{JobFactory, JobService};
use crate::server::config::Config;
use crate::server::storage::{Reader, SessionRef, Submit, Writer};
use compact::{COMPACT_MESSAGE, CompactionJob};

/// The session's compaction instruction; empty until the slot exists.
pub(crate) fn instruction_of(inner: &Inner, session: i64) -> String {
    inner
        .slots
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&session)
        .and_then(|handle| handle.slot().as_ref().map(|slot| slot.instruction.clone()))
        .unwrap_or_default()
}

/// The registry: book, deliver, interrupt, drop. The api holds one handle.
pub struct Register {
    inner: Arc<Inner>,
}

pub struct Inner {
    pub(crate) cfg: Config,
    pub(crate) writer: Writer,
    pub(crate) mcp: Mutex<Option<Arc<dyn McpBridge>>>,
    pub(crate) jobs: Arc<JobFactory>,
    pub(crate) events: Sender<Event>,
    pub(crate) slots: Mutex<HashMap<i64, Arc<Handle>>>,
}

/// One message for one session; the api encodes, the loop decodes and runs.
#[derive(Debug, Clone)]
pub enum Message {
    /// A user turn.
    Submit {
        prefix: Option<String>,
        text: String,
        suffix: Option<String>,
    },
    /// A compaction turn.
    Compact {
        retention: Option<Retention>,
        focus: Option<String>,
    },
    /// A kairos wake: text only, no front-end subscriber.
    Wake { text: String },
}

/// The loop-side retention: the loop owns the summary machine, so it owns the type.
#[derive(Debug, Clone, Copy)]
pub enum Retention {
    /// A token budget; whole blocks are claimed newest-first while they fit.
    Tokens(usize),
    /// A fixed number of newest blocks.
    Blocks(usize),
}

impl Retention {
    fn cut(self) -> Option<cut::Retention> {
        match self {
            Retention::Tokens(n) => Some(cut::Retention::Tokens(n)),
            Retention::Blocks(n) => Some(cut::Retention::Blocks(n)),
        }
    }
}

impl Register {
    /// The registry takes the writer and the config; nothing above this layer writes again.
    pub fn new(writer: Writer, cfg: Config, events: Sender<Event>) -> Register {
        Register {
            inner: Arc::new(Inner {
                mcp: Mutex::new(mcp_bridge(&cfg)),
                jobs: Arc::new(JobFactory::new(
                    cfg.app.server.agent.tools.kairos.clone(),
                    Arc::new(outlets::NoWake),
                )),
                cfg,
                writer,
                events,
                slots: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Deliver one message; `Some(turn)` = the round it opened.
    pub fn deliver(&self, session: i64, msg: Message) -> Result<Option<i64>> {
        self.shell(session).deliver(msg)
    }

    /// The session's handle, booked empty on first use.
    fn shell(&self, session: i64) -> Arc<Handle> {
        let mut slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots
            .entry(session)
            .or_insert_with(|| {
                Arc::new(Handle {
                    inner: Arc::downgrade(&self.inner),
                    id: session,
                    slot: Mutex::new(None),
                })
            })
            .clone()
    }

    /// Set the stop flag on a running turn; no-op when idle.
    pub fn interrupt(&self, session: i64) {
        let Ok(handle) = self.handle(session) else {
            return;
        };
        if let Some(flag) = handle.slot().as_ref().and_then(|slot| slot.stop.clone()) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Drop a session's runtime pieces. Rewind does not come through here.
    pub fn drop(&self, id: i64) {
        let Some(handle) = self
            .inner
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
        else {
            return;
        };
        if let Some(slot) = handle.slot().as_ref() {
            slot.cancel_jobs();
        }
    }

    /// One session-scoped state change: `model`, `cwd`, or `profile`.
    pub fn set(&self, session: i64, field: &str, value: &str) -> Result<()> {
        let handle = self.handle(session)?;
        match field {
            "profile" => {
                let instruction = build::instruction(&self.inner.cfg, value)?;
                let mut slot = handle.slot();
                let slot = slot.get_or_insert_with(|| Slot::new(instruction, value.to_string()));
                slot.pending = Some(value.to_string());
                Ok(())
            }
            "model" => {
                let mut slot = handle.slot();
                if let Some(live) = slot.as_mut().and_then(|slot| slot.live.as_mut()) {
                    let cfg = self.inner.cfg.clone();
                    live.transport =
                        build::transport(&cfg, session, &build::model_of(&cfg, value)?)?;
                }
                Ok(())
            }
            "cwd" => {
                let mut slot = handle.slot();
                if let Some(live) = slot.as_mut().and_then(|slot| slot.live.as_mut()) {
                    live.driver.workspace.chdir(std::path::PathBuf::from(value));
                }
                Ok(())
            }
            other => anyhow::bail!("unknown state field: {other}"),
        }
    }

    pub(crate) fn handle(&self, session: i64) -> Result<Arc<Handle>> {
        self.inner
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&session)
            .cloned()
            .context("session is not registered")
    }
}

fn mcp_bridge(cfg: &Config) -> Option<Arc<dyn McpBridge>> {
    if cfg.mcp.servers.is_empty() {
        return None;
    }
    Some(Arc::new(crate::server::agent::tools::SyncMcpBridge::new(
        cfg.mcp.clone(),
    )))
}

/// The per-session state, behind the Handle.
pub(crate) struct Slot {
    pub(crate) live: Option<Live>,
    pub(crate) pending: Option<String>,
    pub(crate) land: bool,
    pub(crate) instruction: String,
    pub(crate) profile: String,
    pub(crate) wakes: Arc<Mutex<Vec<String>>>,
    pub(crate) jobs: Option<Arc<JobService>>,
    pub(crate) stop: Option<Arc<AtomicBool>>,
}

impl Slot {
    pub(crate) fn new(instruction: String, profile: String) -> Self {
        Self {
            live: None,
            pending: None,
            land: true,
            instruction,
            profile,
            wakes: Arc::new(Mutex::new(Vec::new())),
            jobs: None,
            stop: None,
        }
    }

    pub(crate) fn cancel_jobs(&self) {
        if let Some(jobs) = &self.jobs {
            jobs.cancel_all();
        }
    }
}

pub(crate) struct Handle {
    pub(crate) inner: Weak<Inner>,
    pub(crate) id: i64,
    slot: Mutex<Option<Slot>>,
}

impl Handle {
    pub(crate) fn upgrade(&self) -> Result<Arc<Inner>> {
        self.inner.upgrade().context("the registry is gone")
    }

    pub(crate) fn slot(&self) -> std::sync::MutexGuard<'_, Option<Slot>> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn deliver(&self, msg: Message) -> Result<Option<i64>> {
        match msg {
            Message::Wake { text } => self.wake(text),
            Message::Submit {
                prefix,
                text,
                suffix,
            } => self.turn(prefix, text, suffix).map(Some),
            Message::Compact { retention, focus } => self.compact(retention, focus).map(Some),
        }
    }

    pub(crate) fn wake(&self, text: String) -> Result<Option<i64>> {
        let inner = self.upgrade()?;
        let mut slot = self.slot();
        if slot.as_ref().is_some_and(|slot| slot.stop.is_some()) {
            if let Some(slot) = slot.as_ref() {
                slot.wakes
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(text);
            }
            return Ok(None);
        }
        let live = self.take_live(&inner)?;
        drop(slot);
        let landed = inner.writer.push(Submit::Wake {
            session: self.id,
            text,
            ts: Some(crate::server::platform::time::now_stamp()),
        })?;
        let turn = landed.turn.context("the writer opened no round")?;
        self.spawn_turn(&inner, turn, live);
        Ok(Some(turn))
    }

    pub(crate) fn turn(
        &self,
        prefix: Option<String>,
        text: String,
        suffix: Option<String>,
    ) -> Result<i64> {
        let inner = self.upgrade()?;
        let mut slot = self.slot();
        if slot.as_ref().is_some_and(|slot| slot.stop.is_some()) {
            anyhow::bail!("the session is already in a turn");
        }
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(slot) = slot.as_mut() {
            slot.stop = Some(stop.clone());
        }
        let live = match self.take_live(&inner) {
            Ok(live) => live,
            Err(e) => {
                if let Some(slot) = slot.as_mut() {
                    slot.stop = None;
                }
                return Err(e);
            }
        };
        drop(slot);
        let landed = inner.writer.push(Submit::UserMessage {
            session: SessionRef::Id(self.id),
            prefix,
            text,
            suffix,
            ts: Some(crate::server::platform::time::now_stamp()),
        });
        let turn = match landed {
            Ok(landed) => landed.turn.context("the writer opened no round")?,
            Err(e) => {
                self.release(live);
                return Err(e);
            }
        };
        self.spawn_turn(&inner, turn, live);
        Ok(turn)
    }

    pub(crate) fn compact(
        &self,
        retention: Option<Retention>,
        focus: Option<String>,
    ) -> Result<i64> {
        let inner = self.upgrade()?;
        let mut slot = self.slot();
        if slot.as_ref().is_some_and(|slot| slot.stop.is_some()) {
            anyhow::bail!("the session is already in a turn");
        }
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(slot) = slot.as_mut() {
            slot.stop = Some(stop.clone());
        }
        let live = match self.take_live(&inner) {
            Ok(live) => live,
            Err(e) => {
                if let Some(slot) = slot.as_mut() {
                    slot.stop = None;
                }
                return Err(e);
            }
        };
        drop(slot);
        let job = match compact::plan(&inner, &live, self.id, retention, focus, stop) {
            Ok(job) => job,
            Err(e) => {
                self.release(live);
                return Err(e);
            }
        };
        let landed = inner.writer.push(Submit::UserMessage {
            session: SessionRef::Id(self.id),
            prefix: None,
            text: COMPACT_MESSAGE.to_string(),
            suffix: None,
            ts: Some(crate::server::platform::time::now_stamp()),
        });
        let turn = match landed {
            Ok(landed) => landed.turn.context("the writer opened no round")?,
            Err(e) => {
                self.release(live);
                return Err(e);
            }
        };
        self.spawn_compaction(&inner, turn, live, job);
        Ok(turn)
    }

    pub(crate) fn take_live(&self, inner: &Inner) -> Result<Live> {
        let conn = Reader::open(&inner.writer.path())?;
        let live = build::build(self, inner, &conn, None);
        Reader::close(conn);
        live
    }

    /// One turn on its own thread.
    pub(crate) fn spawn_turn(&self, inner: &Arc<Inner>, turn: i64, mut live: Live) {
        let writer = inner.writer.clone();
        let db = inner.writer.path();
        let id = self.id;
        let weak = self.inner.clone();
        let mut outlet = EventOutlet::new(inner.events.clone(), id);
        let _ = std::thread::Builder::new()
            .name(format!("turn-{id}"))
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let conn = Reader::open(&db)?;
                    let ended = agent_loop::run(
                        &writer,
                        &conn,
                        &mut live,
                        Turn {
                            session: id,
                            turn_id: turn,
                        },
                        &mut outlet,
                    );
                    Reader::close(conn);
                    ended.map(|_| ())
                })();
                if let Err(e) = &result {
                    crate::server::platform::log::error(
                        "loop",
                        format!("session {id}: the turn died: {e:#}"),
                    );
                }
                settle(&weak, live, id);
            });
    }

    pub(crate) fn spawn_compaction(
        &self,
        inner: &Arc<Inner>,
        _turn: i64,
        live: Live,
        job: CompactionJob,
    ) {
        let writer = inner.writer.clone();
        let id = self.id;
        let weak = self.inner.clone();
        let _ = std::thread::Builder::new()
            .name(format!("compact-{id}"))
            .spawn(move || {
                match compact::run(&writer, &job) {
                    Ok(()) => {
                        if let Some(slot) = Handle::slot_of(&weak, id).as_mut() {
                            slot.land = true;
                        }
                    }
                    Err(e) => {
                        crate::server::platform::log::warn(
                            "loop",
                            format!(
                                "session {id}: the compaction did not land (the context is untouched): {e:#}"
                            ),
                        );
                        let _ = writer.push(Submit::RoundEnd {
                            session: id,
                            stop_reason: None,
                            usage: None,
                        });
                    }
                }
                settle(&weak, live, id);
            });
    }

    /// The unified ending mid-flight: the Live goes back, the claim is gone.
    pub(crate) fn release(&self, live: Live) {
        if let Some(slot) = self.slot().as_mut() {
            slot.stop = None;
            slot.live = Some(live);
        }
    }

    pub(crate) fn slot_of(weak: &Weak<Inner>, id: i64) -> std::sync::MutexGuard<'_, Option<Slot>> {
        let handle = weak
            .upgrade()
            .and_then(|inner| {
                inner
                    .slots
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&id)
                    .cloned()
            })
            .expect("the session was dropped while its turn ran");
        handle.slot()
    }
}

/// The unified ending: claim off, Live kept for the next turn.
pub(crate) fn settle(weak: &Weak<Inner>, live: Live, id: i64) {
    if let Some(handle) = weak.upgrade().and_then(|inner| {
        inner
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned()
    }) {
        let mut slot = handle.slot();
        if let Some(slot) = slot.as_mut() {
            slot.stop = None;
            slot.live = Some(live);
        }
    }
}
