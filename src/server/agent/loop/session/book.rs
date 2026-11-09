//! The registry: one slot per session, and the only way a turn starts.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Weak};

use anyhow::{Context, Result, bail};

use super::change::{self, Change};
use super::handle::Handle;
use super::settings::Settings;
use crate::server::agent::r#loop::job;
use crate::server::agent::r#loop::pen;
use crate::server::agent::r#loop::turn;
use crate::server::agent::r#loop::turn::ask::Ask;
use crate::server::agent::r#loop::turn::cycle::Mode;
use crate::server::agent::request::live::{Element, Event, Stream};
use crate::server::agent::tools::contract::call::Batch;
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
    /// The launch directory the front end handed `init`; a change door never writes it.
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
#[derive(Clone)]
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
            .map(|root| root.to_path_buf())
            .unwrap_or_default()
            .join("mcp.json");
        let batch = crate::server::agent::tools::engine::Engine::new(
            crate::server::agent::tools::contract::env::Limits { lines, bytes },
            cfg.bash_timeout(),
            cfg.subagent_budget(),
            source.clone(),
            export,
            crate::server::agent::tools::contract::env::Paths {
                agents: project.agents,
                answers: project.answers,
                global_agents: cfg
                    .paths()
                    .global_root()
                    .map(|root| root.to_path_buf())
                    .unwrap_or_default()
                    .join("agents"),
            },
        );
        if let Ok(table) =
            crate::server::agent::tools::mcp::setup::servers(&project_mcp, &global_mcp)
            && !table.is_empty()
        {
            source.attach_mcp(Arc::new(
                crate::server::agent::tools::mcp::sync::SyncMcpBridge::new(table),
            ));
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
    pub fn todo(
        &self,
        session: i64,
    ) -> Result<Vec<crate::server::agent::tools::memo::todo::state::TodoItem>> {
        crate::server::agent::tools::memo::todo::replay::current(
            self.inner.source.as_ref(),
            session,
        )
    }

    /// Open a session. It stays lazy until the first message arrives.
    pub fn book(&self, birth: Birth) -> Result<i64> {
        let id = birth.session;
        let transcript = Arc::new(super::history::Transcript::open(
            &self.inner.writer.path(),
            id,
        )?);
        let handle = Arc::new(Handle::new(birth, transcript));
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

    /// Deliver one message: a turn opens now, or the call fails. Nothing is queued here.
    pub fn deliver(&self, session: i64, msg: Message) -> Result<i64> {
        if Source::child(self.inner.source.as_ref(), session).is_some() {
            bail!("session {session} is a sub-session; it takes no messages");
        }
        let handle = self.handle(session)?;
        if handle.busy.load(Ordering::Acquire) {
            bail!("session {} is already in a turn", handle.id);
        }
        start_turn(&self.inner, &handle, start_of(msg), false)
    }

    /// The front end's slot: written whole, taken by the loop, cleared by the front end.
    pub fn write_mail(&self, session: i64, msg: Message) -> Result<()> {
        self.handle(session)?.mailbox.write(msg);
        Ok(())
    }

    pub fn clear_mail(&self, session: i64) -> Result<()> {
        self.handle(session)?.mailbox.del();
        Ok(())
    }

    pub fn interrupt(&self, session: i64) {
        if let Ok(handle) = self.handle(session) {
            handle.interrupt();
        }
    }

    pub fn drop(&self, id: i64) {
        unbook(&self.inner, id);
    }

    /// A rewind rewrote this session's list: drop the cache and read it again.
    pub fn forget(&self, session: i64) {
        if let Ok(handle) = self.handle(session)
            && let Err(e) = handle.transcript().reload()
        {
            crate::server::platform::log::warn(
                "loop",
                format!("session {session}: history reload failed: {e:#}"),
            );
        }
    }

    /// Apply one change; a `Cwd` change answers the directory it landed on.
    pub fn change(&self, session: i64, change: Change) -> Result<Option<std::path::PathBuf>> {
        let handle = self.handle(session)?;
        change::apply(
            &handle,
            change,
            &self.inner.writer,
            self.inner.settings.scrooge,
        )
    }

    /// Is this session on the table?
    pub fn booked(&self, session: i64) -> bool {
        self.inner.slots.get(session).is_some()
    }

    /// The running turn's tally and the model it runs on, from one handle.
    pub fn running(&self, session: i64) -> Option<(super::handle::Partial, (String, String))> {
        let handle = self.handle(session).ok()?;
        let partial = handle.partial()?;
        Some((partial, handle.model()))
    }

    fn handle(&self, session: i64) -> Result<Arc<Handle>> {
        self.inner
            .slots
            .get(session)
            .with_context(|| format!("session {session} is not booked"))
    }
}

pub(crate) fn book_child(inner: &Arc<Inner>, birth: Birth, start: String) -> Result<Arc<Handle>> {
    let transcript = Arc::new(super::history::Transcript::open(
        &inner.writer.path(),
        birth.session,
    )?);
    let handle = Arc::new(Handle::new(birth, transcript));
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
        false,
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
        // `interrupt` is a no-op on an idle session; the scheduler is parked either way.
        handle.mailbox.notify();
    }
    // A finished session keeps no browser tabs: the work and search tabs belong to the session's
    // life, not to the browser's.
    #[cfg(feature = "web")]
    crate::web::close_session(id);
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
        let Some(next) = next_start(&handle) else {
            continue;
        };
        let queued = next.queued();
        let start = next.start();
        let Some(inner) = weak.upgrade() else {
            break;
        };
        if !handle.busy.load(Ordering::Acquire) {
            if let Err(e) = start_turn(&inner, &handle, start.clone(), queued) {
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

fn start_turn(
    inner: &Arc<Inner>,
    handle: &Arc<Handle>,
    start: pen::turn::Start,
    queued: bool,
) -> Result<i64> {
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
    // A new turn spends its own tokens; the tally of the one before it is already in the rows.
    handle.clear_partial();
    let first = handle.transcript().blank();
    let wake = match &start {
        pen::turn::Start::Wake { text } => Some(text.clone()),
        _ => None,
    };
    let mode = mode_of(&start);
    let live = handle.take_live(&inner.dir);
    let cwd = live.cwd.read().unwrap_or_else(|e| e.into_inner()).clone();
    let model = Some((live.provider.clone(), live.model.clone()));
    let (turn, opening) = match pen::turn::open(&inner.writer, handle.id, start, &cwd, model) {
        Ok(pair) => pair,
        Err(e) => {
            handle.busy.store(false, Ordering::Release);
            return Err(e);
        }
    };
    handle.transcript().append_user(opening);
    handle.transcript().started();
    if queued {
        announce(inner, handle.id, turn);
    }
    if let Some(text) = wake {
        let _ = inner.out.send(Event {
            session: handle.id,
            element: Element::Stream(Stream::Wake(text)),
        });
    }
    let inner = inner.clone();
    let thread_handle = handle.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("turn-{}", thread_handle.id))
        .spawn(move || {
            let mut live = live;
            let mut current = turn;
            let mut mode = mode;
            let mut first_turn = first;
            loop {
                let outcome = turn::cycle::run(
                    &inner,
                    &thread_handle,
                    current,
                    &mut live,
                    mode.clone(),
                    first_turn,
                );
                first_turn = false;
                match outcome {
                    Ok(turn::Stop::User) => {
                        if thread_handle.gone.load(Ordering::Acquire) {
                            break;
                        }
                        let Some(next) = next_start(&thread_handle) else {
                            break;
                        };
                        let queued = next.queued();
                        let start = next.start();
                        thread_handle.stop.store(false, Ordering::Release);
                        mode = mode_of(&start);
                        let cwd = live.cwd.read().unwrap_or_else(|e| e.into_inner()).clone();
                        let model = Some((live.provider.clone(), live.model.clone()));
                        match pen::turn::open(&inner.writer, thread_handle.id, start, &cwd, model) {
                            Ok((next, opening)) => {
                                thread_handle.transcript().append_user(opening);
                                if queued {
                                    announce(&inner, thread_handle.id, next);
                                }
                                current = next;
                            }
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
        } => handle.mailbox.write(Message::Submit {
            prefix,
            text,
            suffix,
        }),
        pen::turn::Start::Compact { retention, focus } => {
            handle.mailbox.write(Message::Compact { retention, focus });
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

/// One message as the turn it opens.
fn start_of(msg: Message) -> pen::turn::Start {
    match msg {
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
    }
}

/// Tell the api that the slot turned into this turn, so its door is named after the slot.
fn announce(inner: &Arc<Inner>, session: i64, turn: i64) {
    let _ = inner.out.send(Event {
        session,
        element: Element::Mailbox { turn },
    });
}

/// What the scheduler takes next: a wake text, or the front end's one message.
enum Next {
    Wake(String),
    Send(Message),
}

impl Next {
    /// `true` = this came from the front end's slot.
    fn queued(&self) -> bool {
        matches!(self, Next::Send(_))
    }

    fn start(self) -> pen::turn::Start {
        match self {
            Next::Wake(text) => pen::turn::Start::Wake { text },
            Next::Send(msg) => start_of(msg),
        }
    }
}

fn next_start(handle: &Handle) -> Option<Next> {
    if let Some(text) = handle.mailbox.take_wake() {
        return Some(Next::Wake(text));
    }
    handle.mailbox.take_send().map(Next::Send)
}
