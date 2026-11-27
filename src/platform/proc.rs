// One command's process tree: a kernel container with a leader, taken down as a whole.

use std::io;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::platform::log;

// How long a polite stop gets on Unix before the whole group is killed.
#[cfg(unix)]
const GRACE: Duration = Duration::from_secs(2);

// How long the killer waits for the tree to disappear after the last signal.
pub(crate) const KILL_LIMIT: Duration = Duration::from_millis(50);

// How many creations may queue behind the single fork thread.
const QUEUE: usize = 32;

// Where a tree came from; a fixed word carried only by logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Tool,
    Request,
    Browser,
    Mcp,
    Frontend,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Tool => "tool",
            Origin::Request => "request",
            Origin::Browser => "browser",
            Origin::Mcp => "mcp",
            Origin::Frontend => "frontend",
        }
    }
}

// Whether the tree follows the main process or outlives it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Follow {
    Main,
    Detached,
}

impl Follow {
    fn as_str(self) -> &'static str {
        match self {
            Follow::Main => "main",
            Follow::Detached => "detached",
        }
    }
}

// A tree: the leader's name, its streams, and the platform handle that owns what it starts.
#[derive(Clone)]
pub struct Process(Arc<Inner>);

struct Inner {
    pid: u32,
    origin: Origin,
    follow: Follow,
    io: Mutex<Io>,
    group: sys::Group,
    exit: ExitCell,
    stop_phase: Mutex<StopPhase>,
    stop_failed: AtomicBool,
}

struct Io {
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
}

// What the single waiter published: a status, or the text of the failure.
#[derive(Clone)]
enum Exit {
    Status(ExitStatus),
    Message(String),
}

// The published exit, read repeatedly by any number of callers.
struct ExitCell {
    state: Mutex<Option<Exit>>,
    bell: Condvar,
}

impl ExitCell {
    fn new() -> ExitCell {
        ExitCell {
            state: Mutex::new(None),
            bell: Condvar::new(),
        }
    }

    fn published(&self) -> bool {
        lock(&self.state).is_some()
    }

    fn publish(&self, result: io::Result<ExitStatus>) {
        let value = match result {
            Ok(status) => Exit::Status(status),
            Err(e) => Exit::Message(e.to_string()),
        };
        *lock(&self.state) = Some(value);
        self.bell.notify_all();
    }

    fn peek(&self) -> io::Result<Option<ExitStatus>> {
        match &*lock(&self.state) {
            None => Ok(None),
            Some(Exit::Status(status)) => Ok(Some(*status)),
            Some(Exit::Message(text)) => Err(io::Error::other(text.clone())),
        }
    }

    fn wait(&self) -> io::Result<ExitStatus> {
        let mut state = lock(&self.state);
        loop {
            match &*state {
                Some(Exit::Status(status)) => return Ok(*status),
                Some(Exit::Message(text)) => return Err(io::Error::other(text.clone())),
                None => state = self.bell.wait(state).unwrap_or_else(|e| e.into_inner()),
            }
        }
    }

    fn wait_timeout(&self, limit: Duration) -> io::Result<Option<ExitStatus>> {
        let deadline = Instant::now() + limit;
        let mut state = lock(&self.state);
        loop {
            match &*state {
                Some(Exit::Status(status)) => return Ok(Some(*status)),
                Some(Exit::Message(text)) => return Err(io::Error::other(text.clone())),
                None => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Ok(None);
                    }
                    state = self
                        .bell
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            }
        }
    }
}

// Where a stop stands; written under `Inner.stop_phase` before the platform act.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(windows, allow(dead_code))]
enum Phase {
    #[default]
    None,
    Requesting,
    Terminating,
}

// What the polite request achieved, when the platform has one.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(windows, allow(dead_code))]
enum Grace {
    #[default]
    None,
    Within,
    Ignored,
}

#[derive(Clone, Copy, Default)]
struct StopPhase {
    phase: Phase,
    grace: Grace,
}

impl Process {
    // Start a command as the leader of a fresh tree that follows the main process.
    pub fn spawn(origin: Origin, cmd: &mut Command) -> io::Result<Process> {
        submit(origin, Follow::Main, cmd)
    }

    // Start a tree that outlives the front end; only the front end may ask for one.
    pub fn spawn_detached(origin: Origin, cmd: &mut Command) -> io::Result<Process> {
        if origin != Origin::Frontend {
            log::warn("proc", "detached trees serve the front end only");
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "detached trees serve the front end only",
            ));
        }
        submit(origin, Follow::Detached, cmd)
    }

    // The leader's pid; the tree is the unit, this is only its name in a log.
    pub fn id(&self) -> u32 {
        self.0.pid
    }

    pub fn take_stdin(&self) -> Option<ChildStdin> {
        lock(&self.0.io).stdin.take()
    }

    pub fn take_stdout(&self) -> Option<ChildStdout> {
        lock(&self.0.io).stdout.take()
    }

    pub fn take_stderr(&self) -> Option<ChildStderr> {
        lock(&self.0.io).stderr.take()
    }

    // Stop the tree; the platform decides whether the request is polite first.
    pub fn stop(&self) {
        sys::stop(self);
    }

    pub fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        self.0.exit.peek()
    }

    pub fn wait(&self) -> io::Result<ExitStatus> {
        self.0.exit.wait()
    }

    pub fn wait_timeout(&self, limit: Duration) -> io::Result<Option<ExitStatus>> {
        self.0.exit.wait_timeout(limit)
    }
}

impl std::fmt::Debug for Process {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Process").field("pid", &self.0.pid).finish()
    }
}

fn lock<T>(cell: &Mutex<T>) -> MutexGuard<'_, T> {
    cell.lock().unwrap_or_else(|e| e.into_inner())
}

// One creation task, handed to the only thread allowed to call `spawn`.
struct Job {
    cmd: Command,
    origin: Origin,
    follow: Follow,
    reply: SyncSender<io::Result<Process>>,
}

// The single creation point's handle; it never exits, so the parent-death binding is the main process.
struct Forker {
    tx: SyncSender<Job>,
}

static FORKER: OnceLock<Result<Forker, String>> = OnceLock::new();

fn forker() -> io::Result<&'static Forker> {
    match FORKER.get_or_init(start_forker) {
        Ok(forker) => Ok(forker),
        Err(reason) => Err(io::Error::other(format!(
            "process spawner unavailable: {reason}"
        ))),
    }
}

fn start_forker() -> Result<Forker, String> {
    let (tx, rx) = sync_channel::<Job>(QUEUE);
    std::thread::Builder::new()
        .name("process-fork".into())
        .spawn(move || {
            while let Ok(job) = rx.recv() {
                let Job {
                    cmd,
                    origin,
                    follow,
                    reply,
                } = job;
                let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    create(cmd, origin, follow)
                }))
                .unwrap_or_else(|_| Err(io::Error::other("process creation panicked")));
                let _ = reply.send(made);
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Forker { tx })
}

fn submit(origin: Origin, follow: Follow, cmd: &mut Command) -> io::Result<Process> {
    let forker = forker()?;
    let (reply, answer) = sync_channel(1);
    let job = Job {
        cmd: take(cmd),
        origin,
        follow,
        reply,
    };
    forker
        .tx
        .send(job)
        .map_err(|_| io::Error::other("process spawner is gone"))?;
    answer
        .recv()
        .map_err(|_| io::Error::other("process spawner dropped the reply"))?
}

// Move the command to the fork thread, leaving the caller with an inert one.
fn take(cmd: &mut Command) -> Command {
    std::mem::replace(cmd, Command::new(cmd.get_program()))
}

// Runs on the fork thread only: the whole birth of one tree, from exec to its waiter.
fn create(mut cmd: Command, origin: Origin, follow: Follow) -> io::Result<Process> {
    sys::prepare(&mut cmd, follow);
    let mut child = cmd.spawn()?;
    let pid = child.id();
    let io = Io {
        stdin: child.stdin.take(),
        stdout: child.stdout.take(),
        stderr: child.stderr.take(),
    };
    let group = sys::Group::attach(pid, follow);
    let backoff = group.backed_off();
    if backoff {
        log::warn(
            "proc",
            format!("pid={pid} single-process stop, tree guarantee lost"),
        );
    }
    let inner = Arc::new(Inner {
        pid,
        origin,
        follow,
        io: Mutex::new(io),
        group,
        exit: ExitCell::new(),
        stop_phase: Mutex::new(StopPhase::default()),
        stop_failed: AtomicBool::new(false),
    });
    // The leader was born suspended so that its job existed before its first instruction.
    if let Err(reason) = sys::resume(pid) {
        log::error("proc", format!("pid={pid} resume failed: {reason}"));
        Process(inner).stop();
        return Err(io::Error::other(reason));
    }
    let container = if backoff { "backoff" } else { sys::CONTAINER };
    log::info(
        "proc",
        format!(
            "spawn origin={} pid={pid} follow={} container={container} backoff={}",
            origin.as_str(),
            follow.as_str(),
            if backoff { "yes" } else { "no" }
        ),
    );
    if let Err(e) = spawn_reaper(inner.clone(), child) {
        Process(inner).stop();
        return Err(e);
    }
    Ok(Process(inner))
}

// One waiter per tree: it owns the Child until the leader exits, then reclaims and publishes.
fn spawn_reaper(inner: Arc<Inner>, mut child: Child) -> io::Result<()> {
    let pid = inner.pid;
    std::thread::Builder::new()
        .name(format!("process-reap-{pid}"))
        .spawn(move || {
            let status = child.wait();
            let reclaimed = inner.group.reclaim();
            let failed = inner.stop_failed.load(Ordering::SeqCst);
            let code = status
                .as_ref()
                .ok()
                .and_then(|status| status.code())
                .map(|code| code.to_string())
                .unwrap_or_else(|| "none".to_string());
            inner.exit.publish(status);
            // Read after publishing: the stop side writes the final grace when its wait returns.
            let phase = *lock(&inner.stop_phase);
            let method = method_of(phase, reclaimed);
            log::info(
                "proc",
                format!(
                    "exit origin={} pid={} follow={} method={method} grace={} code={code} reclaim={}{}",
                    inner.origin.as_str(),
                    pid,
                    inner.follow.as_str(),
                    grace_str(phase.grace),
                    if reclaimed { "yes" } else { "no" },
                    if failed { " failed=yes" } else { "" }
                ),
            );
        })
        .map(|_| ())
}

fn method_of(phase: StopPhase, reclaimed: bool) -> &'static str {
    if reclaimed {
        return "reclaimed";
    }
    match phase.phase {
        Phase::None => "natural",
        Phase::Requesting => "request_stop",
        Phase::Terminating if phase.grace == Grace::Within => "request_stop",
        Phase::Terminating => "kill_tree",
    }
}

fn grace_str(grace: Grace) -> &'static str {
    match grace {
        Grace::None => "none",
        Grace::Within => "within",
        Grace::Ignored => "ignored",
    }
}

#[cfg(unix)]
#[path = "proc/sys_unix.rs"]
mod sys;

#[cfg(windows)]
#[path = "proc/sys_windows.rs"]
mod sys;
