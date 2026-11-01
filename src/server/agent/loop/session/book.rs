//! The registry: one slot per session, and the only way a turn starts.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Weak};

use anyhow::{Context, Result, bail};

use super::change::{self, Change};
use super::handle::Handle;
use super::mailbox::Mailbox;
use super::settings::Settings;
use crate::server::agent::r#loop::job;
use crate::server::agent::r#loop::pen;
use crate::server::agent::r#loop::turn;
use crate::server::agent::r#loop::turn::ask::Ask;
use crate::server::agent::r#loop::turn::batch::Batch;
use crate::server::agent::r#loop::turn::cycle::Mode;
use crate::server::agent::request::live::Event;
use crate::server::agent::tools::contract::view::Source;
use crate::server::storage::Writer;

/// What a caller needs to open one session. The caller resolves the profile
/// or the session's meta row; the loop never reads either.
#[derive(Debug, Clone)]
pub struct Birth {
    pub session: i64,
    pub provider: String,
    pub model: String,
    pub system: String,
    /// The session's tool names; manuals are expanded per request.
    pub tools: Vec<String>,
    /// The compaction instruction template; `{{focus}}` marks where the focus goes.
    pub compact: String,
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

pub use crate::server::agent::request::contract::Retention;

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
    pub(crate) source: Arc<super::source::Db>,
    pub(crate) dir: PathBuf,
    pub(crate) slots: Arc<super::jobs::Slots>,
    pub(crate) factory: job::Factory,
}

impl Loop {
    pub fn new(
        writer: Writer,
        settings: Settings,
        ask: Arc<dyn Ask>,
        out: Sender<Event>,
        cfg: &crate::server::config::Config,
    ) -> Self {
        let path = writer.path();
        let source = super::source::Db::new(path.clone());
        let dir = cfg.export_dir();
        let export = Arc::new(super::export::Files::new(path, dir.clone()));
        let (lines, bytes) = cfg.limits();
        let project = cfg.project_paths();
        let project_mcp = project.mcp.clone();
        let global_mcp = cfg
            .paths()
            .global_root()
            .unwrap_or_default()
            .join("mcp.json");
        let batch = crate::server::agent::tools::Engine::new(
            crate::server::agent::tools::contract::env::Limits { lines, bytes },
            cfg.bash_timeout(),
            cfg.subagent_budget(),
            source.clone(),
            source.clone(),
            export,
            crate::server::agent::tools::contract::env::Paths {
                project: project.project,
                agents: project.agents,
                answers: project.answers,
                global_agents: cfg.paths().global_root().unwrap_or_default().join("agents"),
                mcp: project_mcp.clone(),
                global_mcp: global_mcp.clone(),
            },
        );
        if let Ok(table) =
            crate::server::agent::tools::mcp::setup::servers(&project_mcp, &global_mcp)
            && !table.is_empty()
        {
            source.attach_mcp(Arc::new(crate::server::agent::tools::SyncMcpBridge::new(
                table,
            )));
        }
        let slots = super::jobs::Slots::new();
        let factory = job::Factory::new(super::jobs::values(cfg), slots.clone());
        Self {
            inner: Arc::new(Inner {
                writer,
                settings,
                out,
                ask,
                batch: Arc::new(batch),
                source,
                dir,
                slots,
                factory,
            }),
        }
    }

    /// The list a session's `todo` calls add up to: the switch, then a replay. The api door hands
    /// this over as a front end's starting point.
    pub fn todo(&self, session: i64) -> Result<Vec<crate::server::agent::tools::TodoItem>> {
        crate::server::agent::tools::todo_current(self.inner.source.as_ref(), session)
    }

    /// Open a session. It stays lazy until the first message arrives.
    pub fn book(&self, birth: Birth) -> Result<i64> {
        let id = birth.session;
        let handle = Arc::new(Handle::new(birth));
        if !self.inner.slots.insert(id, handle.clone()) {
            bail!("session {id} is already booked");
        }
        self.inner
            .source
            .attach_jobs(id, self.inner.factory.make(id));
        if let Err(e) = spawn_scheduler(&self.inner, handle) {
            self.inner.slots.remove(id);
            self.inner.source.detach_jobs(id);
            return Err(e);
        }
        Ok(id)
    }

    /// Deliver one message. A busy session queues it for its current turn.
    pub fn deliver(&self, session: i64, msg: Message) -> Result<Option<i64>> {
        if Source::child(self.inner.source.as_ref(), session).is_some() {
            bail!("session {session} is a sub-session; it takes no messages");
        }
        let handle = self.handle(session)?;
        if handle.busy.load(Ordering::Acquire) {
            if matches!(&msg, Message::Compact { .. }) {
                bail!("session {} is already in a turn", handle.id);
            }
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
        unbook(&self.inner, id);
    }

    pub fn change(&self, session: i64, change: Change) -> Result<()> {
        let handle = self.handle(session)?;
        let birth = handle.birth();
        let tools_json = serde_json::to_string(&turn::cycle::manuals(&birth.tools))?;
        change::apply(
            &handle,
            change,
            &self.inner.writer,
            self.inner.settings.scrooge,
            &tools_json,
        )
    }

    pub fn mailbox(&self, session: i64) -> Result<Arc<Mailbox>> {
        Ok(self.handle(session)?.mailbox.clone())
    }

    fn handle(&self, session: i64) -> Result<Arc<Handle>> {
        self.inner
            .slots
            .get(session)
            .with_context(|| format!("session {session} is not booked"))
    }
}

pub(crate) fn book_child(inner: &Arc<Inner>, birth: Birth, start: String) -> Result<Arc<Handle>> {
    if let Some(meta) = birth.meta {
        inner.writer.push(crate::server::storage::Submit {
            rows: vec![crate::server::storage::Row::Meta {
                session: birth.session,
                meta: Some(crate::server::storage::Ref::Known(meta)),
            }],
        })?;
    }
    let handle = Arc::new(Handle::new(birth));
    if !inner.slots.insert(handle.id, handle.clone()) {
        bail!("session {} is already booked", handle.id);
    }
    if let Err(e) = spawn_scheduler(inner, handle.clone()) {
        inner.slots.remove(handle.id);
        return Err(e);
    }
    start_turn(
        inner,
        &handle,
        pen::turn::Start::User {
            prefix: None,
            text: start,
            suffix: None,
        },
    )?;
    Ok(handle)
}

/// Take a session off the table: its job service, its child state, its handle.
pub(crate) fn unbook(inner: &Arc<Inner>, id: i64) {
    inner.source.detach_jobs(id);
    inner.source.detach_child(id);
    if let Some(handle) = inner.slots.remove(id) {
        handle.gone.store(true, Ordering::Release);
        handle.interrupt();
    }
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
            if let Err(e) = start_turn(&inner, &handle, start.clone()) {
                crate::server::platform::log::error(
                    "loop",
                    format!("session {}: queued mail did not start: {e:#}", handle.id),
                );
                requeue(&handle, start);
            }
        } else {
            requeue(&handle, start);
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
    let live = handle.take_live(&inner.dir);
    let cwd = live.cwd.read().unwrap_or_else(|e| e.into_inner()).clone();
    let turn = match pen::turn::open(&inner.writer, handle.id, start, &cwd) {
        Ok(turn) => turn,
        Err(e) => {
            handle.busy.store(false, Ordering::Release);
            return Err(e);
        }
    };
    let inner = inner.clone();
    let thread_handle = handle.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("turn-{}", thread_handle.id))
        .spawn(move || {
            let mut live = live;
            let mut current = turn;
            let mut mode = mode;
            loop {
                match turn::cycle::run(&inner, &thread_handle, current, &mut live, mode.clone()) {
                    Ok(turn::Stop::User) => {
                        if thread_handle.gone.load(Ordering::Acquire) {
                            break;
                        }
                        let Some(start) = next_start(&thread_handle) else {
                            break;
                        };
                        thread_handle.stop.store(false, Ordering::Release);
                        mode = mode_of(&start);
                        let cwd = live.cwd.read().unwrap_or_else(|e| e.into_inner()).clone();
                        match pen::turn::open(&inner.writer, thread_handle.id, start, &cwd) {
                            Ok(next) => current = next,
                            Err(e) => {
                                crate::server::platform::log::error(
                                    "loop",
                                    format!(
                                        "session {}: follow-up did not open: {e:#}",
                                        thread_handle.id
                                    ),
                                );
                                break;
                            }
                        }
                    }
                    Ok(_) => break,
                    Err(e) => {
                        crate::server::platform::log::error(
                            "loop",
                            format!("session {}: turn {current} died: {e:#}", thread_handle.id),
                        );
                        break;
                    }
                }
            }
            thread_handle.put_live(live);
            thread_handle.stop.store(false, Ordering::Release);
            thread_handle.busy.store(false, Ordering::Release);
            thread_handle.mailbox.notify();
        });
    if let Err(e) = spawned {
        handle.busy.store(false, Ordering::Release);
        return Err(e.into());
    }
    Ok(turn)
}

fn requeue(handle: &Handle, start: pen::turn::Start) {
    match start {
        pen::turn::Start::Wake { text } => handle.mailbox.push_wake(text),
        pen::turn::Start::User {
            prefix,
            text,
            suffix,
        } => handle.mailbox.push_user(Message::Submit {
            prefix,
            text,
            suffix,
        }),
        pen::turn::Start::Compact { retention, focus } => {
            handle
                .mailbox
                .push_user(Message::Compact { retention, focus });
        }
    }
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
