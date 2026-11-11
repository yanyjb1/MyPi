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
use crate::server::agent::tools::contract::call::{Batch, Cwd};
use crate::server::agent::tools::contract::env::{Aliases, Defaults};
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
    /// The session's tool names; manuals are built once, from `registry::roster`.
    pub tools: Vec<String>,
    /// The request prefix's tool section: built when the session is booked, never rebuilt.
    pub manuals: Arc<Vec<crate::server::agent::request::message::ToolDef>>,
    /// The compaction instruction template; `{{focus}}` marks where the focus goes.
    pub compact: String,
    pub meta: Option<i64>,
    /// The launch directory the front end handed `init`; a change door never writes it.
    pub cwd: PathBuf,
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
    pub(crate) slots: Arc<super::jobs::Slots>,
    /// What every session's tool side starts from.
    pub(crate) defaults: Defaults,
    /// One sub-session's deadline; `task` never borrows kairos' numbers.
    pub(crate) task_timeout: std::time::Duration,
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
        let export = Arc::new(super::export::Files::new(path.clone(), cfg.export_dir()));
        let (lines, bytes) = cfg.limits();
        let global_mcp = cfg
            .paths()
            .global_root()
            .map(|root| root.to_path_buf())
            .unwrap_or_default()
            .join("mcp.json");
        let project = cfg.project_paths();
        let slots = super::jobs::Slots::new();
        let factory = job::Factory::new(super::jobs::values(cfg), slots.clone());
        let source = super::source::Db::new(path.clone(), global_mcp, factory);
        let defaults = Defaults {
            limits: crate::server::agent::tools::contract::env::Limits { lines, bytes },
            bash: crate::server::agent::tools::contract::env::BashEnv {
                timeout: cfg.bash_timeout(),
                shell: crate::server::platform::shell::Shell::default(),
            },
            aliases: cfg.aliases(),
            default_budget: cfg.subagent_budget(),
            export,
            paths: crate::server::agent::tools::contract::env::Paths {
                agents: project.agents,
                answers: project.answers,
                global_agents: cfg
                    .paths()
                    .global_root()
                    .map(|root| root.to_path_buf())
                    .unwrap_or_default()
                    .join("agents"),
            },
            source: source.clone(),
        };
        Self {
            inner: Arc::new(Inner {
                writer,
                settings,
                out,
                ask,
                batch: Arc::new(crate::server::agent::tools::engine::Engine),
                source,
                slots,
                defaults,
                task_timeout: cfg.task_timeout(),
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
        bind(&self.inner, birth)?;
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
        let Ok(handle) = self.handle(session) else {
            return;
        };
        if let Err(e) = handle.transcript().reload() {
            crate::server::platform::log::warn(
                "loop",
                format!("session {session}: history reload failed: {e:#}"),
            );
        }
        // A rewind cuts the rows behind aliases; the tool side holds a snapshot, so read it again.
        match self.inner.source.aliases(session) {
            Ok(aliases) => {
                handle.env().aliases = Aliases::new(self.inner.defaults.aliases.clone(), aliases);
            }
            Err(e) => crate::server::platform::log::warn(
                "loop",
                format!("session {session}: alias reload failed: {e:#}"),
            ),
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

/// One session on the table: its two sides built from the birth, then its scheduler.
pub(crate) fn bind(inner: &Arc<Inner>, birth: Birth) -> Result<Arc<Handle>> {
    let transcript = Arc::new(super::history::Transcript::open(
        &inner.writer.path(),
        birth.session,
    )?);
    let cwd: Cwd = Arc::new(std::sync::RwLock::new(birth.cwd.clone()));
    let stop = crate::server::platform::stop::Stop::new();
    let env = inner
        .defaults
        .session(birth.session, cwd.clone(), birth.tools.clone(), stop)?;
    let handle = Arc::new(Handle::new(birth, env, cwd, transcript));
    if !inner.slots.insert(handle.id, handle.clone()) {
        bail!("session {} is already booked", handle.id);
    }
    if let Err(e) = spawn_scheduler(inner, handle.clone()) {
        inner.slots.remove(handle.id);
        return Err(e);
    }
    Ok(handle)
}

pub(crate) fn book_child(inner: &Arc<Inner>, birth: Birth, start: String) -> Result<Arc<Handle>> {
    let handle = bind(inner, birth)?;
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
    inner.source.detach_mcp(id);
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
        handle.mark_idle();
        bail!("session {} is gone", handle.id);
    }
    handle.stop.reset();
    // A new turn spends its own tokens; the tally of the one before it is already in the rows.
    handle.clear_partial();
    let first = handle.transcript().blank();
    let wake = match &start {
        pen::turn::Start::Wake { text } => Some(text.clone()),
        _ => None,
    };
    let mode = mode_of(&start);
    let cwd = handle.cwd();
    let model = Some(handle.model());
    let (turn, opening) = match pen::turn::open(&inner.writer, handle.id, start, &cwd, model) {
        Ok(pair) => pair,
        Err(e) => {
            handle.mark_idle();
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
            // The handle goes back to idle however this thread ends, a panic included.
            let _guard = TurnGuard {
                handle: thread_handle.clone(),
            };
            let mut current = turn;
            let mut mode = mode;
            let mut first_turn = first;
            loop {
                let outcome =
                    turn::cycle::run(&inner, &thread_handle, current, mode.clone(), first_turn);
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
                        thread_handle.stop.reset();
                        mode = mode_of(&start);
                        let cwd = thread_handle.cwd();
                        let model = Some(thread_handle.model());
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
        });
    if let Err(e) = spawned {
        handle.mark_idle();
        return Err(e.into());
    }
    Ok(turn)
}

/// A turn thread's share of the handle: the one exit that turns it back to idle.
struct TurnGuard {
    handle: Arc<Handle>,
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        self.handle.stop.reset();
        self.handle.mark_idle();
    }
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
