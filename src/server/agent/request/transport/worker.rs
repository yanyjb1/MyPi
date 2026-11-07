use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    Stream {
        id: u64,
        url: String,
        body: serde_json::Value,
        headers: Vec<(String, String)>,
        timeout_ms: u64,
        first_ms: u64,
        idle_ms: u64,
    },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    /// Startup handshake: the worker is ready. `Command::spawn` succeeding does not mean it works.
    Ready,
    Chunk {
        id: u64,
        line: String,
    },
    Done {
        id: u64,
    },
    /// One failure. `status`/`body` are set only for HTTP status errors, for `HttpFailure`.
    Error {
        id: u64,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<String>,
    },
}

#[derive(Debug)]
pub struct HttpWorker {
    child: Child,
    pid: Arc<AtomicU32>,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next_id: u64,
}
pub type SharedWorker = Arc<Mutex<HttpWorker>>;

/// The environment variable that turns this executable into a worker.
pub const WORKER_ENV: &str = "MYPI_HTTP_WORKER";

/// One session's worker: the child that owns its sockets, and whether a request is in flight.
pub struct SessionWorker {
    slot: Mutex<Option<SharedWorker>>,
    pid: Arc<AtomicU32>,
    in_flight: AtomicBool,
    /// An interrupt landed before the request it meant: the next request starts on a fresh child.
    pending: AtomicBool,
}

impl SessionWorker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            slot: Mutex::new(None),
            pid: Arc::new(AtomicU32::new(0)),
            in_flight: AtomicBool::new(false),
            pending: AtomicBool::new(false),
        })
    }

    /// The child for one request, spawned on first use.
    pub fn request(&self) -> Option<SharedWorker> {
        if self.pending.swap(false, Ordering::AcqRel) {
            self.restart();
        }
        let shared = self.slot()?;
        self.in_flight.store(true, Ordering::Release);
        Some(shared)
    }

    /// The child, spawned on first use.
    fn slot(&self) -> Option<SharedWorker> {
        let mut slot = self.slot.lock().ok()?;
        if slot.is_none() {
            let fresh = HttpWorker::spawn().ok()?;
            self.pid
                .store(HttpWorker::pid(&fresh).unwrap_or(0), Ordering::Release);
            *slot = Some(fresh);
        }
        slot.clone()
    }

    /// The request is over, one way or another.
    pub fn end(&self) {
        self.in_flight.store(false, Ordering::Release);
    }

    /// No request in flight: the interrupt may still be one moment early.
    pub fn interrupt(&self) {
        if self.in_flight.load(Ordering::Acquire) {
            self.restart();
        } else {
            self.pending.store(true, Ordering::Release);
        }
    }

    /// Kill the child that owns the sockets, then take a fresh one in its place.
    pub fn restart(&self) {
        self.in_flight.store(false, Ordering::Release);
        let pid = self.pid.swap(0, Ordering::AcqRel);
        if pid != 0 {
            HttpWorker::kill_pid(pid);
        }
        let fresh = HttpWorker::spawn().ok();
        if let Some(worker) = fresh.as_ref()
            && let Some(pid) = HttpWorker::pid(worker)
        {
            self.pid.store(pid, Ordering::Release);
        }
        if let Ok(mut slot) = self.slot.lock() {
            let old = std::mem::replace(&mut *slot, fresh);
            if let Some(old) = old {
                std::thread::spawn(move || {
                    if let Ok(mut old) = old.lock() {
                        old.reap();
                    }
                });
            }
        }
    }
}

impl Drop for SessionWorker {
    fn drop(&mut self) {
        let pid = self.pid.swap(0, Ordering::AcqRel);
        if pid != 0 {
            HttpWorker::kill_pid(pid);
        }
        if let Ok(mut slot) = self.slot.lock()
            && let Some(worker) = slot.take()
            && let Ok(mut worker) = worker.lock()
        {
            let _ = worker.child.kill();
            let _ = worker.child.wait();
        }
    }
}

impl HttpWorker {
    pub fn pid(slot: &SharedWorker) -> Option<u32> {
        slot.lock()
            .ok()
            .map(|w| w.pid.load(Ordering::Relaxed))
            .filter(|p| *p != 0)
    }
    pub fn kill_pid(pid: u32) {
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status();
    }
    pub fn spawn() -> Result<SharedWorker> {
        let exe = std::env::current_exe().context("cannot locate the mypi executable")?;
        let mut child = Command::new(exe)
            .env(WORKER_ENV, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to start the HTTP worker")?;
        let pid = Arc::new(AtomicU32::new(child.id()));
        let input = child
            .stdin
            .take()
            .context("HTTP worker stdin unavailable")?;
        let output = BufReader::new(
            child
                .stdout
                .take()
                .context("HTTP worker stdout unavailable")?,
        );
        // Handshake: is the child really running a worker? A silent child is not ours to wait for.
        let output = match greet(output, HELLO_LIMIT) {
            Ok(output) => output,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        Ok(Arc::new(Mutex::new(Self {
            child,
            pid,
            input,
            output,
            next_id: 1,
        })))
    }
    fn send(&mut self, req: &Request) -> Result<()> {
        serde_json::to_writer(&mut self.input, req)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn stream<F>(
        &mut self,
        url: &str,
        body: serde_json::Value,
        headers: &[(String, String)],
        timeout_ms: u64,
        first_ms: u64,
        idle_ms: u64,
        mut on_line: F,
    ) -> Result<()>
    where
        F: FnMut(&str) -> bool,
    {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&Request::Stream {
            id,
            url: url.into(),
            body,
            headers: headers.to_vec(),
            timeout_ms,
            first_ms,
            idle_ms,
        })?;
        let mut raw = String::new();
        loop {
            raw.clear();
            let n = self.output.read_line(&mut raw)?;
            if n == 0 {
                anyhow::bail!("HTTP worker exited")
            }
            let msg: Response = serde_json::from_str(raw.trim_end())?;
            match msg {
                Response::Chunk { id: got, line } if got == id => {
                    if !on_line(&line) {
                        anyhow::bail!("HTTP stream interrupted")
                    }
                }
                Response::Done { id: got } if got == id => return Ok(()),
                Response::Error {
                    id: got,
                    message,
                    status,
                    body,
                } if got == id => {
                    // status/body travel with the death report, so the parent can rebuild `HttpFailure`.
                    match (status, body) {
                        (Some(s), Some(b)) => anyhow::bail!("__HTTP__{s}\n{b}"),
                        _ => anyhow::bail!("{message}"),
                    }
                }
                _ => {}
            }
        }
    }
    pub fn reap(&mut self) {
        let _ = self.child.wait();
    }
}

pub fn run() -> Result<()> {
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    // Greet first: this line is how the parent knows the child is really running a worker.
    serde_json::to_writer(&mut out, &Response::Ready)?;
    out.write_all(b"\n")?;
    out.flush()?;
    let mut agent: Option<(u64, ureq::Agent)> = None;
    let mut raw = String::new();
    while {
        raw.clear();
        input.read_line(&mut raw)? != 0
    } {
        let req: Request = serde_json::from_str(raw.trim_end())?;
        match req {
            Request::Stream {
                id,
                url,
                body,
                headers,
                timeout_ms,
                first_ms,
                idle_ms,
            } => {
                if agent.as_ref().map(|(ms, _)| *ms) != Some(timeout_ms) {
                    agent = Some((timeout_ms, agent_for(timeout_ms)));
                }
                let client = &agent.as_ref().expect("just built").1;
                let mut req = client.post(&url);
                for (name, value) in &headers {
                    req = req.header(name, value);
                }
                match req.send_json(&body) {
                    Ok(response) => {
                        let status = response.status().as_u16();
                        if (200..300).contains(&status) {
                            let body = response.into_body().into_reader();
                            if let Err(e) = stream_body(body, id, first_ms, idle_ms, &mut out) {
                                let _ = write_error(&mut out, id, e.to_string());
                            }
                            let _ = write_done(&mut out, id);
                        } else {
                            // A status error: read the body (empty if unreadable; the status code must survive).
                            let text = response
                                .into_body()
                                .read_to_string()
                                .unwrap_or_else(|_| String::new());
                            let _ = write_status(&mut out, id, status, text);
                        }
                    }
                    Err(e) => {
                        let _ = write_error(&mut out, id, e.to_string());
                    }
                }
            }
        }
    }
    Ok(())
}

/// The greeting deadline: a worker that never speaks is taken down.
const HELLO_LIMIT: Duration = Duration::from_secs(10);

/// Read the worker's greeting line, giving up after `limit`.
fn greet(mut output: BufReader<ChildStdout>, limit: Duration) -> Result<BufReader<ChildStdout>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let n = std::io::BufRead::read_line(&mut output, &mut line).unwrap_or(0);
        let ready = n > 0
            && matches!(
                serde_json::from_str::<Response>(line.trim()),
                Ok(Response::Ready)
            );
        let _ = tx.send((ready, output));
    });
    match rx.recv_timeout(limit) {
        Ok((true, output)) => Ok(output),
        Ok((false, _)) => anyhow::bail!("HTTP worker answered something else"),
        Err(_) => anyhow::bail!("HTTP worker never became ready"),
    }
}

fn agent_for(timeout_ms: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_millis(timeout_ms)))
        .timeout_recv_body(None)
        .http_status_as_error(false)
        .build()
        .into()
}

/// One response body, line by line, with the silence deadline the decoder no longer owns.
fn stream_body(
    body: impl std::io::Read + Send + 'static,
    id: u64,
    first_ms: u64,
    idle_ms: u64,
    out: &mut impl Write,
) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<String>>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(body);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let text = line.trim_end_matches(['\r', '\n']).to_string();
                    if tx.send(Ok(text)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                    break;
                }
            }
        }
    });
    let mut first = true;
    loop {
        let wait = if first { first_ms } else { idle_ms };
        match rx.recv_timeout(Duration::from_millis(wait)) {
            Ok(Ok(line)) => {
                serde_json::to_writer(&mut *out, &Response::Chunk { id, line })?;
                out.write_all(b"\n")?;
                out.flush()?;
                first = false;
            }
            Ok(Err(e)) => {
                write_error(&mut *out, id, e.to_string())?;
                return Ok(());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = write_error(
                    &mut *out,
                    id,
                    format!("no event from upstream within {wait} ms"),
                );
                let _ = out.flush();
                // The body reader is blocked on the socket; only the process can drop it.
                std::process::exit(1);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn write_done(out: &mut impl Write, id: u64) -> Result<()> {
    serde_json::to_writer(&mut *out, &Response::Done { id })?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

fn write_error(out: &mut impl Write, id: u64, message: String) -> Result<()> {
    serde_json::to_writer(
        &mut *out,
        &Response::Error {
            id,
            message,
            status: None,
            body: None,
        },
    )?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

fn write_status(out: &mut impl Write, id: u64, status: u16, body: String) -> Result<()> {
    serde_json::to_writer(
        &mut *out,
        &Response::Error {
            id,
            message: format!("HTTP {status}"),
            status: Some(status),
            body: Some(body),
        },
    )?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

/// The child's stdout, one whole line at a time.
pub(crate) struct ChannelReader {
    pub(crate) rx: std::sync::mpsc::Receiver<String>,
    pub(crate) pending: Vec<u8>,
}

impl std::io::Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pending.is_empty() {
            // A closed channel is the end of the body: failures arrive as a marker line first.
            let Ok(line) = self.rx.recv() else {
                return Ok(0);
            };
            self.pending.extend_from_slice(line.as_bytes());
            self.pending.push(b'\n');
        }
        let n = out.len().min(self.pending.len());
        out[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}
