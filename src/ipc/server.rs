// The server's half of the wire: the database lock, the listener, one reader and one writer per
// connection. Every frame, including hello and bye, goes to `crate::server::api::Api`.

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::ipc::contract::Frame;
use crate::ipc::endpoint::{self, Endpoint};
use crate::ipc::line::{self, Lines};
use crate::ipc::stream;
use crate::ipc::transport::{self, Listener, Stream};
use crate::server::api::Api;

/// A connection is watched for silence this long before the server may stop.
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

/// Serve until the api says every front end is gone.
pub fn serve(bound: Bound) -> Result<()> {
    let Bound {
        listener,
        endpoint,
        endpoint_file,
        api,
        claim: _claim,
    } = bound;
    let stopping = Arc::new(AtomicBool::new(false));

    {
        let api = Arc::clone(&api);
        let stopping = Arc::clone(&stopping);
        std::thread::Builder::new()
            .name("ipc-accept".to_string())
            .spawn(move || accept_loop(listener, api, stopping))
            .context("cannot start the accept loop")?;
    }

    while !api.idle_for(GRACE) {
        std::thread::sleep(Duration::from_millis(250));
    }
    stopping.store(true, Ordering::Relaxed);
    transport::unlink(&endpoint);
    endpoint::remove(&endpoint_file);
    Ok(())
}

/// `--server` in one step: claim, bind, serve.
pub fn run(api: Api, db: &Path, endpoint: Endpoint, claim: Claim) -> Result<()> {
    serve(bind(Arc::new(api), db, endpoint, claim)?)
}

fn accept_loop(listener: Listener, api: Arc<Api>, stopping: Arc<AtomicBool>) {
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
        if let Err(e) = std::thread::Builder::new()
            .name(format!("ipc-conn-{next}"))
            .spawn(move || handle(stream, api, next))
        {
            eprintln!("mypi: cannot start connection {next}: {e}");
        }
    }
}

fn handle(stream: Stream, api: Arc<Api>, id: u64) {
    let (conn, queue) = api.connect();
    let out = BufWriter::new(stream.clone());
    if let Err(e) = std::thread::Builder::new()
        .name(format!("ipc-write-{id}"))
        .spawn(move || stream::write(out, queue))
    {
        eprintln!("mypi: cannot start connection {id}'s writer: {e}");
        api.disconnect(&conn);
        return;
    }

    let mut lines = Lines::new(BufReader::new(stream));
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
        if let Some(answer) = api.receive(&conn, frame)
            && !conn.send(answer)
        {
            break;
        }
    }

    api.disconnect(&conn);
    drop(conn);
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
