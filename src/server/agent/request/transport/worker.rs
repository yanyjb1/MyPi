use std::sync::atomic::{AtomicU32, Ordering};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    Stream {
        id: u64,
        base_url: String,
        api_key: String,
        body: serde_json::Value,
        /// Extra headers the parent computed. The worker does not know what they mean, it sends them.
        #[serde(default)]
        headers: Vec<(String, String)>,
    },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    /// Startup handshake: the worker is ready. `Command::spawn` succeeding does not mean it works.
    Ready,
    Chunk { id: u64, line: String },
    Done { id: u64 },
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

impl HttpWorker {
    pub fn pid(slot: &SharedWorker) -> Option<u32> {
        slot.lock().ok().map(|w| w.pid.load(Ordering::Relaxed)).filter(|p| *p != 0)
    }
    pub fn kill_pid(pid: u32) {
        let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).status();
    }
    pub fn replace(slot: &SharedWorker) -> Result<()> {
        let fresh = Self::spawn()?;
        let mut old = slot.lock().map_err(|_| anyhow::anyhow!("HTTP worker lock poisoned"))?;
        let mut new = fresh.lock().map_err(|_| anyhow::anyhow!("HTTP worker lock poisoned"))?;
        let _ = old.child.kill();
        let _ = old.child.wait();
        std::mem::swap(&mut *old, &mut *new);
        Ok(())
    }

    pub fn spawn() -> Result<SharedWorker> { 
        let exe = std::env::current_exe().context("cannot locate the mypi executable")?;
        let mut child = Command::new(exe)
            .arg("--http-worker")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().context("failed to start the HTTP worker")?;
        let pid = Arc::new(AtomicU32::new(child.id()));
        let input = child.stdin.take().context("HTTP worker stdin unavailable")?;
        let mut output = BufReader::new(child.stdout.take().context("HTTP worker stdout unavailable")?);
        // Handshake: is the child really running a worker? If not, take it down and return Err.
        let mut hello = String::new();
        let ok = std::io::BufRead::read_line(&mut output, &mut hello).is_ok()
            && matches!(
                serde_json::from_str::<Response>(hello.trim()),
                Ok(Response::Ready)
            );
        if !ok {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("HTTP worker never became ready");
        }
        Ok(Arc::new(Mutex::new(Self { child, pid, input, output, next_id: 1 })))
    }
    fn send(&mut self, req: &Request) -> Result<()> {
        serde_json::to_writer(&mut self.input, req)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        Ok(())
    }
    pub fn stream<F>(
        &mut self,
        base_url: &str,
        api_key: &str,
        body: serde_json::Value,
        headers: &[(String, String)],
        mut on_line: F,
    ) -> Result<()>
    where F: FnMut(&str) -> bool {
        let id = self.next_id; self.next_id += 1;
        self.send(&Request::Stream {
            id,
            base_url: base_url.into(),
            api_key: api_key.into(),
            body,
            headers: headers.to_vec(),
        })?;
        let mut raw = String::new();
        loop {
            raw.clear();
            let n = self.output.read_line(&mut raw)?;
            if n == 0 { anyhow::bail!("HTTP worker exited") }
            let msg: Response = serde_json::from_str(raw.trim_end())?;
            match msg {
                Response::Chunk { id: got, line } if got == id => {
                    if !on_line(&line) { anyhow::bail!("HTTP stream interrupted") }
                }
                Response::Done { id: got } if got == id => return Ok(()),
                Response::Error { id: got, message, status, body } if got == id => {
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
    pub fn kill_and_replace(slot: &SharedWorker) -> Result<()> {
        let mut old = slot.lock().map_err(|_| anyhow::anyhow!("HTTP worker lock poisoned"))?;
        let _ = old.child.kill();
        let _ = old.child.wait();
        let fresh = match Self::spawn() {
            Ok(f) => f,
            // The replacement would not start: swap the dead worker for an empty slot (later requests go direct).
            Err(e) => {
                let mut empty = Self::make_empty();
                std::mem::swap(&mut *old, &mut empty);
                return Err(e.context("HTTP worker replacement failed; direct connection restored"));
            }
        };
        let mut replacement = fresh.lock().map_err(|_| anyhow::anyhow!("HTTP worker lock poisoned"))?;
        std::mem::swap(&mut *old, &mut *replacement);
        Ok(())
    }

    /// An "empty slot": the replacement on a failed spawn, an immediately-exiting `/bin/true`.
    fn make_empty() -> Self {
        let mut child = Command::new("true")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn()
            .expect("`/bin/true` always spawns");
        let pid = Arc::new(AtomicU32::new(child.id()));
        let input = child.stdin.take().expect("piped stdin");
        let output = BufReader::new(child.stdout.take().expect("piped stdout"));
        Self { child, pid, input, output, next_id: 1 }
    }
    pub fn reap(&mut self) { let _ = self.child.wait(); }
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
    let mut raw = String::new();
    while { raw.clear(); input.read_line(&mut raw)? != 0 } {
        let req: Request = serde_json::from_str(raw.trim_end())?;
        match req {
            Request::Stream { id, base_url, api_key, body, headers } => {
                // Same as the direct path: non-2xx is not an error, the body has to be read.
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .timeout_connect(Some(std::time::Duration::from_secs(30)))
                    .timeout_recv_response(Some(std::time::Duration::from_secs(120)))
                    .http_status_as_error(false)
                    .build().into();
                let mut req = agent
                    .post(format!("{}/chat/completions", base_url.trim_end_matches('/')))
                    .header("Authorization", &format!("Bearer {api_key}"));
                for (name, value) in &headers {
                    req = req.header(name, value);
                }
                let resp = req.send_json(&body);
                match resp {
                    Ok(response) => {
                        let status = response.status().as_u16();
                        if (200..300).contains(&status) {
                            let mut reader = BufReader::new(response.into_body().into_reader());
                            let mut line = String::new();
                            loop {
                                line.clear();
                                match reader.read_line(&mut line) {
                                    Ok(0) => break,
                                    Ok(_) => { serde_json::to_writer(&mut out, &Response::Chunk { id, line: line.trim_end_matches(['\r','\n']).into() })?; out.write_all(b"\n")?; out.flush()?; }
                                    Err(e) => { serde_json::to_writer(&mut out, &Response::Error { id, message: e.to_string(), status: None, body: None })?; out.write_all(b"\n")?; break; }
                                }
                            }
                            serde_json::to_writer(&mut out, &Response::Done { id })?; out.write_all(b"\n")?; out.flush()?;
                        } else {
                            // A status error: read the body (empty if unreadable; the status code must survive).
                            let text = response
                                .into_body()
                                .read_to_string()
                                .unwrap_or_else(|_| String::new());
                            serde_json::to_writer(&mut out, &Response::Error { id, message: format!("HTTP {status}"), status: Some(status), body: Some(text) })?;
                            out.write_all(b"\n")?; out.flush()?;
                        }
                    }
                    Err(e) => { serde_json::to_writer(&mut out, &Response::Error { id, message: e.to_string(), status: None, body: None })?; out.write_all(b"\n")?; out.flush()?; }
                }
            }
        }
    }
    Ok(())
}
