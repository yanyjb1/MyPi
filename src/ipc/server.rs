//! The server's half of the wire: the database lock, the listener, the peers.
//! Requests go to [`crate::server::api`]; an empty peer table is when it stops.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::server::api::{Opened, Subscribe};
use crate::server::config::Config;
use crate::server::platform::ipc::{self, Cost, Listener, Stream};
use crate::server::platform::log;

use super::frame::{self, Lines};
use super::{Answer, Ask, socket_path, stream};

/// How long an empty peer table must stay empty: the window between a front
/// end being started and its `Hello` arriving.
const GRACE: Duration = Duration::from_millis(300);

/// The database lock, held while this process is the server.
pub struct Claim(#[allow(dead_code)] File);

/// Try to become this database's server. `None` = somebody else already is.
pub fn claim(db: &Path) -> Result<Option<Claim>> {
    // The database's home may not exist yet, and the lock needs it.
    if let Some(dir) = db.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(db)
        .with_context(|| format!("cannot open {}", db.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Claim(file))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => {
            Err(e).with_context(|| format!("cannot lock {}", db.display()))
        }
    }
}

/// A listener that is ready to take front ends.
pub struct Bound {
    listener: Listener,
    cost: Cost,
    path: PathBuf,
    cfg: Config,
    claim: Claim,
}

/// Create the endpoint; the caller already holds the lock. Binding comes
/// before a front end is started (see the entry point).
pub fn bind(cfg: Config, db: &Path, claim: Claim) -> Result<Bound> {
    let cost = cfg.app.server.platform.ipc.cost();
    let path = socket_path(db);
    clean_stale(cost, &path);
    let listener =
        ipc::bind(cost, &path).with_context(|| format!("cannot listen on {}", path.display()))?;
    set_private(&path);
    // Whoever started the server is looking at this terminal: say where it is.
    println!("mypi: listening on {}", path.display());
    log::info("ipc", format!("listening on {}", path.display()));
    Ok(Bound {
        listener,
        cost,
        path,
        cfg,
        claim,
    })
}

/// Serve until the peer table empties.
pub fn serve(bound: Bound) -> Result<()> {
    let Bound {
        listener,
        cost,
        path,
        cfg,
        claim: _claim,
    } = bound;
    let peers = Arc::new(Peers::default());
    let stopping = Arc::new(AtomicBool::new(false));

    {
        let peers = Arc::clone(&peers);
        let stopping = Arc::clone(&stopping);
        std::thread::Builder::new()
            .name("ipc-accept".to_string())
            .spawn(move || accept_loop(listener, cfg, peers, stopping))
            .context("cannot start the accept loop")?;
    }

    while !peers.empty_after_grace(GRACE) {}
    stopping.store(true, Ordering::Relaxed);
    ipc::unlink(cost, &path);
    log::info("ipc", "the last front end is gone; stopping");
    Ok(())
}

/// `--server` in one step: claim, bind, serve.
pub fn run(cfg: Config, db: &Path, claim: Claim) -> Result<()> {
    serve(bind(cfg, db, claim)?)
}

/// Accept one front end after another, each on a thread of its own.
fn accept_loop(listener: Listener, cfg: Config, peers: Arc<Peers>, stopping: Arc<AtomicBool>) {
    let mut next = 0u64;
    loop {
        if stopping.load(Ordering::Relaxed) {
            return;
        }
        let stream = match listener.accept() {
            Ok(stream) => stream,
            Err(e) => {
                log::warn("ipc", format!("accept: {e}"));
                return;
            }
        };
        next += 1;
        let cfg = cfg.clone();
        let peers = Arc::clone(&peers);
        if let Err(e) = std::thread::Builder::new()
            .name(format!("ipc-conn-{next}"))
            .spawn(move || handle(stream, cfg, peers, next))
        {
            log::error("ipc", format!("cannot start connection {next}: {e}"));
        }
    }
}

/// One connection: read a line, carry it out, write the answer. A connection
/// that sent `Hello` is a peer and is counted until it leaves.
fn handle(stream: Stream, cfg: Config, peers: Arc<Peers>, id: u64) {
    let mut lines = Lines::new(BufReader::new(stream.clone()));
    let mut out = Some(BufWriter::new(stream));
    let mut registered = false;

    loop {
        let line = match lines.next_line() {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                log::info("ipc", format!("connection {id}: {e}"));
                break;
            }
        };
        let ask = match frame::decode::<Ask>(line) {
            Ok(ask) => ask,
            Err(e) => {
                log::warn("ipc", format!("connection {id}: {e}"));
                break;
            }
        };
        let writer = out.as_mut().expect("the writer is taken only to become a stream");
        match ask {
            Ask::Hello { .. } => {
                if !registered {
                    registered = true;
                    peers.joined();
                }
            }
            Ask::Bye => break,
            Ask::Call { call } => {
                let answer = crate::server::api::call::answer(&cfg, call)
                    .map(|reply| Answer::Reply { reply })
                    .unwrap_or_else(|e| Answer::error(&e));
                if reply(writer, &answer).is_err() {
                    break;
                }
            }
            Ask::Post { post } => {
                let answer = crate::server::api::post::apply(&cfg, post)
                    .map(|done| Answer::Done { done })
                    .unwrap_or_else(|e| Answer::error(&e));
                if reply(writer, &answer).is_err() {
                    break;
                }
            }
            Ask::Subscribe { subscribe } => match subscribe {
                Subscribe::Watch { viewer, session } => {
                    let watching = crate::server::api::subscribe::watch(&cfg, viewer, session);
                    match watching {
                        Ok(rx) => {
                            // This connection is a stream from here on, and it
                            // is read no further.
                            let out = out.take().expect("just checked");
                            stream::pump(Box::new(out), rx, cfg, viewer, session);
                        }
                        Err(e) => {
                            let _ = reply(writer, &Answer::error(&e));
                        }
                    }
                    break;
                }
                other => {
                    let answer = open(&cfg, other)
                        .map(|opened| Answer::Opened { opened })
                        .unwrap_or_else(|e| Answer::error(&e));
                    if reply(writer, &answer).is_err() {
                        break;
                    }
                }
            },
        }
    }

    if registered {
        peers.left();
    }
}

/// Open a stretch: a round, or a compaction.
fn open(cfg: &Config, subscribe: Subscribe) -> Result<Opened> {
    match subscribe {
        Subscribe::Submit {
            session,
            prefix,
            text,
            suffix,
        } => crate::server::api::subscribe::submit(
            cfg,
            session,
            prefix.as_deref(),
            &text,
            suffix.as_deref(),
        ),
        Subscribe::Compact {
            session,
            retention,
            focus,
        } => crate::server::api::subscribe::compact(cfg, session, retention, focus),
        Subscribe::Watch { .. } => anyhow::bail!("watch is handled before this"),
    }
}

/// One line out. An error here means the peer left.
fn reply(out: &mut impl Write, answer: &Answer) -> std::io::Result<()> {
    out.write_all(&frame::encode(answer)?)?;
    out.flush()
}

/// Take away the socket a server that is gone left behind.
fn clean_stale(cost: Cost, path: &Path) {
    if !path.exists() {
        return;
    }
    if ipc::connect(cost, path).is_ok() {
        log::warn(
            "ipc",
            format!("something is still answering on {} — binding anyway", path.display()),
        );
    }
    ipc::unlink(cost, path);
}

/// Nobody but this user may reach the socket.
fn set_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            log::warn("ipc", format!("cannot set 0600 on {}: {e}", path.display()));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// How many front ends have said hello and not left. Stream connections never
/// say hello, so watching does not count as being here.
#[derive(Default)]
struct Peers {
    live: Mutex<usize>,
    changed: Condvar,
}

impl Peers {
    /// A front end said hello.
    fn joined(&self) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        *live += 1;
        self.changed.notify_all();
    }

    /// A front end said goodbye, or its connection ended.
    fn left(&self) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        *live = live.saturating_sub(1);
        self.changed.notify_all();
    }

    /// True when the table has been empty for a whole grace period: the wait
    /// covers a front end that has not registered yet.
    fn empty_after_grace(&self, grace: Duration) -> bool {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if *live > 0 {
            // Somebody is here. Sleep until the table changes at all, so the
            // caller does not spin while it waits.
            let _ = self
                .changed
                .wait_timeout(live, Duration::from_millis(250))
                .unwrap_or_else(|e| e.into_inner());
            return false;
        }
        let (guard, waited) = self
            .changed
            .wait_timeout(live, grace)
            .unwrap_or_else(|e| e.into_inner());
        *guard == 0 && waited.timed_out()
    }
}
