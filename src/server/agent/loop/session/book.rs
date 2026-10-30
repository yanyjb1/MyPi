//! The registry: one slot per session, and the only way a turn starts.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, Weak};

use anyhow::{Context, Result, bail};

use super::change::{self, Change};
use super::handle::Handle;
use super::mailbox::Mailbox;
use super::settings::Settings;
use crate::server::agent::r#loop::out::Event;
use crate::server::agent::r#loop::pen;
use crate::server::agent::r#loop::turn;
use crate::server::agent::r#loop::turn::ask::Ask;
use crate::server::agent::r#loop::turn::batch::Batch;
use crate::server::agent::r#loop::turn::cycle::Mode;
use crate::server::storage::Writer;

/// What a caller needs to open one session. The caller resolves the profile
/// or the session's meta row; the loop never reads either.
#[derive(Debug, Clone)]
pub struct Birth {
    pub session: i64,
    pub provider: String,
    pub model: String,
    pub system: String,
    pub tools: Vec<serde_json::Value>,
    pub meta: Option<i64>,
    pub cwd: Option<PathBuf>,
}

/// A message for one session.
#[derive(Debug, Clone)]
pub enum Message {
    Submit {
        prefix: Option<String>,
        text: String,
        suffix: Option<String>,
    },
    Compact {
        retention: Option<Retention>,
        focus: Option<String>,
    },
}

/// How much of the history a compaction keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    Tokens(usize),
    Blocks(usize),
}

/// The loop object. One per process; every session hangs off it.
pub struct Loop {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) writer: Writer,
    pub(crate) settings: Settings,
    pub(crate) out: Sender<Event>,
    pub(crate) ask: Arc<dyn Ask>,
    pub(crate) batch: Arc<dyn Batch>,
    pub(crate) slots: Mutex<HashMap<i64, Arc<Handle>>>,
}

impl Loop {
    pub fn new(
        writer: Writer,
        settings: Settings,
        ask: Arc<dyn Ask>,
        batch: Arc<dyn Batch>,
        out: Sender<Event>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                writer,
                settings,
                out,
                ask,
                batch,
                slots: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Open a session. It stays lazy until the first message arrives.
    pub fn book(&self, birth: Birth) -> Result<i64> {
        let id = birth.session;
        let handle = Arc::new(Handle::new(birth));
        {
            let mut slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
            if slots.contains_key(&id) {
                bail!("session {id} is already booked");
            }
            slots.insert(id, handle.clone());
        }
        spawn_scheduler(&self.inner, handle)?;
        Ok(id)
    }

    /// Deliver one message. A busy session queues it for its current turn.
    pub fn deliver(&self, session: i64, msg: Message) -> Result<Option<i64>> {
        let handle = self.handle(session)?;
        if handle.busy.load(Ordering::Acquire) {
            handle.mailbox.push_user(msg);
            return Ok(None);
        }
        let start = match msg {
            Message::Submit {
                prefix,
                text,
                suffix,
            } => pen::turn::Start::User {
                prefix,
                text,
                suffix,
            },
            Message::Compact { retention, focus } => pen::turn::Start::Compact { retention, focus },
        };
        start_turn(&self.inner, &handle, start).map(Some)
    }

    pub fn interrupt(&self, session: i64) {
        if let Ok(handle) = self.handle(session) {
            handle.interrupt();
        }
    }

    pub fn drop(&self, id: i64) {
        let handle = self
            .inner
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        if let Some(handle) = handle {
            handle.gone.store(true, Ordering::Release);
            handle.interrupt();
        }
    }

    pub fn change(&self, session: i64, change: Change) -> Result<()> {
        let handle = self.handle(session)?;
        change::apply(
            &handle,
            change,
            &self.inner.writer,
            self.inner.settings.scrooge,
        )
    }

    pub fn mailbox(&self, session: i64) -> Result<Arc<Mailbox>> {
        Ok(self.handle(session)?.mailbox.clone())
    }

    fn handle(&self, session: i64) -> Result<Arc<Handle>> {
        self.inner
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&session)
            .cloned()
            .with_context(|| format!("session {session} is not booked"))
    }
}

pub(crate) fn book_child(inner: &Arc<Inner>, birth: Birth, start: String) -> Result<()> {
    if let Some(meta) = birth.meta {
        inner.writer.push(crate::server::storage::Submit {
            rows: vec![crate::server::storage::Row::Meta {
                session: birth.session,
                meta: Some(crate::server::storage::Ref::Known(meta)),
            }],
        })?;
    }
    let handle = Arc::new(Handle::new(birth));
    {
        let mut slots = inner.slots.lock().unwrap_or_else(|e| e.into_inner());
        if slots.contains_key(&handle.id) {
            bail!("session {} is already booked", handle.id);
        }
        slots.insert(handle.id, handle.clone());
    }
    spawn_scheduler(inner, handle.clone())?;
    start_turn(
        inner,
        &handle,
        pen::turn::Start::User {
            prefix: None,
            text: start,
            suffix: None,
        },
    )?;
    Ok(())
}

fn spawn_scheduler(inner: &Arc<Inner>, handle: Arc<Handle>) -> Result<()> {
    let weak = Arc::downgrade(inner);
    std::thread::Builder::new()
        .name(format!("sched-{}", handle.id))
        .spawn(move || schedule(weak, handle))
        .context("cannot start the session scheduler")?;
    Ok(())
}

fn schedule(weak: Weak<Inner>, handle: Arc<Handle>) {
    loop {
        handle.mailbox.wait_any(&handle.gone);
        if handle.gone.load(Ordering::Acquire) {
            break;
        }
        while handle.mailbox.has_any() && handle.busy.load(Ordering::Acquire) {
            if handle.gone.load(Ordering::Acquire) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let Some(start) = next_start(&handle) else {
            continue;
        };
        let Some(inner) = weak.upgrade() else {
            break;
        };
        if !handle.busy.load(Ordering::Acquire) {
            if let Err(e) = start_turn(&inner, &handle, start) {
                crate::server::platform::log::error(
                    "loop",
                    format!("session {}: queued mail did not start: {e:#}", handle.id),
                );
            }
        }
    }
}

fn start_turn(inner: &Arc<Inner>, handle: &Arc<Handle>, start: pen::turn::Start) -> Result<i64> {
    if handle
        .busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        bail!("session {} is already in a turn", handle.id);
    }
    if handle.gone.load(Ordering::Acquire) {
        handle.busy.store(false, Ordering::Release);
        bail!("session {} is gone", handle.id);
    }
    handle.stop.store(false, Ordering::Release);
    let mode = mode_of(&start);
    let live = handle.take_live(&inner.settings);
    let turn = match pen::turn::open(&inner.writer, handle.id, start, &live.cwd) {
        Ok(turn) => turn,
        Err(e) => {
            handle.busy.store(false, Ordering::Release);
            return Err(e);
        }
    };
    let inner = inner.clone();
    let handle = handle.clone();
    std::thread::Builder::new()
        .name(format!("turn-{}", handle.id))
        .spawn(move || {
            let mut live = live;
            let mut current = turn;
            let mut mode = mode;
            loop {
                match turn::cycle::run(&inner, &handle, current, &mut live, mode.clone()) {
                    Ok(turn::Stop::User) => {
                        if handle.gone.load(Ordering::Acquire) {
                            break;
                        }
                        let Some(start) = next_start(&handle) else {
                            break;
                        };
                        handle.stop.store(false, Ordering::Release);
                        mode = mode_of(&start);
                        match pen::turn::open(&inner.writer, handle.id, start, &live.cwd) {
                            Ok(next) => current = next,
                            Err(e) => {
                                crate::server::platform::log::error(
                                    "loop",
                                    format!("session {}: follow-up did not open: {e:#}", handle.id),
                                );
                                break;
                            }
                        }
                    }
                    Ok(_) => break,
                    Err(e) => {
                        crate::server::platform::log::error(
                            "loop",
                            format!("session {}: turn {current} died: {e:#}", handle.id),
                        );
                        break;
                    }
                }
            }
            handle.put_live(live);
            handle.stop.store(false, Ordering::Release);
            handle.busy.store(false, Ordering::Release);
            handle.mailbox.notify();
        })
        .context("cannot start a turn thread")?;
    Ok(turn)
}

fn mode_of(start: &pen::turn::Start) -> Mode {
    match start {
        pen::turn::Start::Compact { retention, focus } => Mode::Compact {
            retention: *retention,
            focus: focus.clone(),
        },
        _ => Mode::Normal,
    }
}

fn next_start(handle: &Handle) -> Option<pen::turn::Start> {
    if let Some(text) = handle.mailbox.take_wake() {
        return Some(pen::turn::Start::Wake { text });
    }
    match handle.mailbox.take_user()? {
        Message::Submit {
            prefix,
            text,
            suffix,
        } => Some(pen::turn::Start::User {
            prefix,
            text,
            suffix,
        }),
        Message::Compact { retention, focus } => {
            Some(pen::turn::Start::Compact { retention, focus })
        }
    }
}
