// The process gate: every external tool the file tools run goes through here. A child leads its own process group, so a stuck child dies with its whole tree instead of holding the call it belongs to.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use crate::platform::proc;
use crate::platform::stop::Stop;

// How many bytes of one stream a child may hand back.
const CAP: usize = 8 << 20;

// What one child handed back.
pub(crate) struct Ran {
    // Its exit code; `None` when a signal ended it or it was killed.
    pub(crate) code: Option<i32>,
    pub(crate) out: String,
    pub(crate) err: String,
    // Whether the deadline killed it.
    pub(crate) timed_out: bool,
    // Whether the turn's stop killed it.
    pub(crate) stopped: bool,
}

// Run one child to its end, or kill its whole group when the deadline passes or the turn is stopped. The caller blocks on the child's exit alone; the deadline and the stop each end that wait by killing the tree, so nothing here looks twice.
pub(crate) fn run(
    mut cmd: Command,
    input: Option<&[u8]>,
    limit: Option<Duration>,
    stop: &Arc<Stop>,
) -> std::io::Result<Ran> {
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let process = proc::Process::spawn(proc::Origin::Tool, &mut cmd)?;

    // The input goes down a thread of its own: a child that reads late must not hold this call.
    if let Some(bytes) = input {
        let mut stdin = process.take_stdin().expect("stdin is piped");
        let owned = bytes.to_vec();
        let _ = std::thread::Builder::new()
            .name("mypi-gate-in".into())
            .spawn(move || {
                let _ = stdin.write_all(&owned);
            });
    }
    let out = drain(process.take_stdout().expect("stdout is piped"));
    let err = drain(process.take_stderr().expect("stderr is piped"));

    // Which side ended the tree: `0` the child itself, `1` the deadline, `2` the turn's stop.
    let killed = Arc::new(AtomicU8::new(0));
    // A stop kills the tree; the blocked wait below returns the moment the leader has gone.
    let hook = {
        let process = process.clone();
        let killed = killed.clone();
        stop.on_stop(move || {
            killed.store(2, Ordering::SeqCst);
            process.stop();
        })
    };
    // The deadline waits on its own bell: the channel goes quiet for good the moment the child ends first.
    let (ended, left) = std::sync::mpsc::channel::<()>();
    if let Some(limit) = limit {
        let process = process.clone();
        let killed = killed.clone();
        let _ = std::thread::Builder::new()
            .name("mypi-gate-deadline".into())
            .spawn(move || {
                if left.recv_timeout(limit) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                    killed.store(1, Ordering::SeqCst);
                    process.stop();
                }
            });
    }
    // The one wait: the child's own exit. The deadline and the stop end it by killing the tree.
    let status = match process.wait() {
        Ok(status) => Some(status),
        Err(e) => {
            process.stop();
            let _ = process.wait_timeout(proc::KILL_LIMIT);
            return Err(e);
        }
    };
    drop(ended);
    drop(hook);
    let timed_out = killed.load(Ordering::SeqCst) == 1;
    let stopped = killed.load(Ordering::SeqCst) == 2;
    Ok(Ran {
        code: status.and_then(|status| status.code()),
        out: out.join().unwrap_or_default(),
        err: err.join().unwrap_or_default(),
        timed_out,
        stopped,
    })
}

// Read one stream to its end on a thread of its own: a child that talks past the pipe's buffer must never block on a reader that is not there yet.
fn drain(pipe: impl std::io::Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::Builder::new()
        .name("mypi-gate-out".into())
        .spawn(move || {
            use std::io::Read as _;
            let mut rdr = std::io::BufReader::new(pipe);
            let mut raw = [0u8; 8192];
            let mut text: Vec<u8> = Vec::new();
            while let Ok(n) = rdr.read(&mut raw) {
                if n == 0 {
                    break;
                }
                if text.len() < CAP {
                    text.extend_from_slice(&raw[..n]);
                }
            }
            String::from_utf8_lossy(&text).to_string()
        })
        .expect("a reader thread")
}

// A program of a toolchain: next to this executable first, then whatever PATH holds.
pub(crate) fn beside(name: &str) -> PathBuf {
    let named = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let near = dir.join(&named);
        if near.is_file() {
            return near;
        }
    }
    PathBuf::from(named)
}
