//! The server's half of the wire: the database lock, the listener, the peers.
//! Requests go to [`crate::server::api::Api`]; an empty peer table is when it stops.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::ipc::contract::{Frame, Kind, Viewer};
use crate::ipc::endpoint::{self, Endpoint};
use crate::ipc::line::{self, Lines};
use crate::ipc::stream;
use crate::ipc::transport::{self, Listener, Stream};
use crate::server::api::{Api, Response};

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
    let mut out: Option<BufWriter<Stream>> = Some(BufWriter::new(stream));
    let mut registered = false;
    let mut viewer: Option<Viewer> = None;

    loop {
        let raw = match lines.next_line() {
            Ok(Some(raw)) => raw,
            Ok(None) => break,
            Err(e) => {
                eprintln!("mypi: ipc connection {id}: {e}");
                break;
            }
        };
        let frame: Frame = match line::decode(&raw) {
            Ok(frame) => frame,
            Err(e) => {
                eprintln!("mypi: ipc connection {id}: {e}");
                break;
            }
        };
        if frame.kind == Kind::Register && frame.name == "hello" {
            match serde_json::from_value::<Viewer>(frame.payload.clone()) {
                Ok(id) => {
                    viewer = Some(id);
                    if !registered {
                        registered = true;
                        peers.joined();
                    }
                }
                Err(_) => break,
            }
            continue;
        }
        let Some(id) = viewer.clone() else {
            continue;
        };
        match api.receive(&id, frame) {
            Response::Frame(answer) => {
                let writer = out.as_mut().expect("the writer is taken only by a stream");
                if write(writer, &answer).is_err() {
                    break;
                }
            }
            Response::Stream(rx) => {
                let writer = out.take().expect("just checked");
                stream::pump(writer, rx, api, id);
                break;
            }
            Response::Silence => {}
        }
    }

    if registered {
        peers.left();
    }
}

fn write(out: &mut impl Write, frame: &Frame) -> std::io::Result<()> {
    let bytes = line::encode(frame)?;
    out.write_all(&bytes).and_then(|()| out.flush())
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
