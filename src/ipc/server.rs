//! The server's half of the wire: the database lock, the listener, the peers.
//! Requests go to [`crate::server::api::Api`]; an empty peer table is when it stops.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::ipc::contract::{Answer, Ask, ErrorCode, Opened, PROTOCOL_VERSION, Subscribe, Viewer};
use crate::ipc::endpoint::{self, Endpoint};
use crate::ipc::frame::{self, Lines};
use crate::ipc::stream;
use crate::ipc::transport::{self, Listener, Stream};
use crate::server::api::Api;

/// How long an empty peer table must stay empty.
const GRACE: Duration = Duration::from_millis(300);

/// The database lock, held while this process is the server.
pub struct Claim(#[allow(dead_code)] File);

/// Try to become this database's server. `None` = somebody else already is.
pub fn claim(db: &Path) -> Result<Option<Claim>> {
    if let Some(dir) = db.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
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
    endpoint: Endpoint,
    endpoint_file: PathBuf,
    api: Arc<Api>,
    claim: Claim,
}

/// Create the endpoint; the caller already holds the lock.
pub fn bind(api: Arc<Api>, db: &Path, endpoint: Endpoint, claim: Claim) -> Result<Bound> {
    let listener =
        transport::bind(&endpoint).with_context(|| format!("cannot listen on {endpoint:?}"))?;
    set_private(&endpoint);
    let endpoint_file = Endpoint::file_path(db);
    endpoint::write(&endpoint_file, &endpoint)
        .with_context(|| format!("cannot write {}", endpoint_file.display()))?;
    println!("mypi: listening on {endpoint:?}");
    Ok(Bound {
        listener,
        endpoint,
        endpoint_file,
        api,
        claim,
    })
}

/// Serve until the peer table empties.
pub fn serve(bound: Bound) -> Result<()> {
    let Bound {
        listener,
        endpoint,
        endpoint_file,
        api,
        claim: _claim,
    } = bound;
    let peers = Arc::new(Peers::default());
    let stopping = Arc::new(AtomicBool::new(false));

    {
        let peers = Arc::clone(&peers);
        let stopping = Arc::clone(&stopping);
        std::thread::Builder::new()
            .name("ipc-accept".to_string())
            .spawn(move || accept_loop(listener, api, peers, stopping))
            .context("cannot start the accept loop")?;
    }

    while !peers.empty_after_grace(GRACE) {}
    stopping.store(true, Ordering::Relaxed);
    transport::unlink(&endpoint);
    endpoint::remove(&endpoint_file);
    Ok(())
}

/// `--server` in one step: claim, bind, serve.
pub fn run(api: Api, db: &Path, endpoint: Endpoint, claim: Claim) -> Result<()> {
    serve(bind(Arc::new(api), db, endpoint, claim)?)
}

fn accept_loop(listener: Listener, api: Arc<Api>, peers: Arc<Peers>, stopping: Arc<AtomicBool>) {
    let mut next = 0u64;
    loop {
        if stopping.load(Ordering::Relaxed) {
            return;
        }
        let stream = match listener.accept() {
            Ok(stream) => stream,
            Err(e) => {
                eprintln!("mypi: ipc accept: {e}");
                return;
            }
        };
        next += 1;
        let api = Arc::clone(&api);
        let peers = Arc::clone(&peers);
        if let Err(e) = std::thread::Builder::new()
            .name(format!("ipc-conn-{next}"))
            .spawn(move || handle(stream, api, peers, next))
        {
            eprintln!("mypi: cannot start connection {next}: {e}");
        }
    }
}

fn handle(stream: Stream, api: Arc<Api>, peers: Arc<Peers>, id: u64) {
    let mut lines = Lines::new(BufReader::new(stream.clone()));
    let mut out = Some(BufWriter::new(stream));
    let mut registered = false;
    let mut viewer: Option<Viewer> = None;

    loop {
        let line = match lines.next_line() {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                eprintln!("mypi: ipc connection {id}: {e}");
                break;
            }
        };
        let ask = match frame::decode::<Ask>(line) {
            Ok(ask) => ask,
            Err(e) => {
                eprintln!("mypi: ipc connection {id}: {e}");
                break;
            }
        };
        let writer = out
            .as_mut()
            .expect("the writer is taken only to become a stream");
        match ask {
            Ask::Hello {
                viewer: id,
                protocol,
            } => {
                if protocol != PROTOCOL_VERSION {
                    let e =
                        anyhow::anyhow!("protocol {protocol} does not match {PROTOCOL_VERSION}");
                    let _ = reply(writer, &Answer::error(ErrorCode::BadRequest, &e));
                    break;
                }
                viewer = Some(id);
                if !registered {
                    registered = true;
                    peers.joined();
                }
            }
            Ask::Bye => break,
            Ask::Call { call } => {
                let Some(id) = viewer.as_ref() else {
                    let e = anyhow::anyhow!("say hello first");
                    if reply(writer, &Answer::error(ErrorCode::NoHello, &e)).is_err() {
                        break;
                    }
                    continue;
                };
                let answer = api
                    .call(id, call)
                    .map(|reply| Answer::Reply { reply })
                    .unwrap_or_else(|e| Answer::error(ErrorCode::Internal, &e));
                if reply(writer, &answer).is_err() {
                    break;
                }
            }
            Ask::Post { post } => {
                let Some(id) = viewer.as_ref() else {
                    let e = anyhow::anyhow!("say hello first");
                    if reply(writer, &Answer::error(ErrorCode::NoHello, &e)).is_err() {
                        break;
                    }
                    continue;
                };
                let answer = api
                    .post(id, post)
                    .map(|done| Answer::Done { done })
                    .unwrap_or_else(|e| Answer::error(ErrorCode::Internal, &e));
                if reply(writer, &answer).is_err() {
                    break;
                }
            }
            Ask::Subscribe { subscribe } => {
                let Some(id) = viewer.as_ref() else {
                    let e = anyhow::anyhow!("say hello first");
                    if reply(writer, &Answer::error(ErrorCode::NoHello, &e)).is_err() {
                        break;
                    }
                    continue;
                };
                match subscribe {
                    Subscribe::Watch { session } => {
                        let rx = api.connect(id);
                        let watching = rx.and_then(|rx| {
                            api.watch(id, session)?;
                            Ok(rx)
                        });
                        match watching {
                            Ok(rx) => {
                                drop(writer);
                                let out = out.take().expect("just checked");
                                stream::pump(Box::new(out), rx, api, id.clone(), session);
                            }
                            Err(e) => {
                                let _ = reply(writer, &Answer::error(ErrorCode::Internal, &e));
                            }
                        }
                        break;
                    }
                    other => {
                        let answer = open(&api, id, other)
                            .map(|opened| Answer::Opened { opened })
                            .unwrap_or_else(|e| Answer::error(ErrorCode::Internal, &e));
                        if reply(writer, &answer).is_err() {
                            break;
                        }
                    }
                }
            }
        }
    }

    if registered {
        peers.left();
    }
}

fn open(api: &Api, viewer: &Viewer, subscribe: Subscribe) -> Result<Opened> {
    match subscribe {
        Subscribe::Submit {
            session,
            prefix,
            text,
            suffix,
        } => api.submit(viewer, session, prefix.as_deref(), &text, suffix.as_deref()),
        Subscribe::Compact {
            session,
            retention,
            focus,
        } => api.compact(viewer, session, retention, focus),
        Subscribe::Watch { .. } => bail!("watch is handled before this"),
    }
}

fn reply(out: &mut impl Write, answer: &Answer) -> std::io::Result<()> {
    out.write_all(&frame::encode(answer)?)?;
    out.flush()
}

fn set_private(endpoint: &Endpoint) {
    #[cfg(unix)]
    if let Endpoint::Unix { path } = endpoint {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            eprintln!("mypi: cannot set 0600 on {}: {e}", path.display());
        }
    }
    #[cfg(not(unix))]
    let _ = endpoint;
}

#[derive(Default)]
struct Peers {
    live: Mutex<usize>,
    changed: Condvar,
}

impl Peers {
    fn joined(&self) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        *live += 1;
        self.changed.notify_all();
    }

    fn left(&self) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        *live = live.saturating_sub(1);
        self.changed.notify_all();
    }

    fn empty_after_grace(&self, grace: Duration) -> bool {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if *live > 0 {
            let _ = self.changed.wait_timeout(live, Duration::from_millis(250));
            return false;
        }
        drop(live);
        std::thread::sleep(grace);
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        *live == 0
    }
}
