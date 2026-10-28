//! One stdio MCP server: process, JSON-RPC, pending calls, stderr tail.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::server::config::McpServerConfig;

pub(crate) struct Server {
    name: String,
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, Sender<Result<Value>>>>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    next_id: u64,
    timeout: Duration,
    closed: Arc<AtomicBool>,
    initialized: bool,
}

pub(crate) fn spawn(name: &str, cfg: &McpServerConfig) -> Result<Server> {
    let mut command = Command::new(cfg.command.as_str());
    command
        .args(cfg.args.iter().map(String::as_str))
        .envs(
            cfg.env
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &cfg.cwd {
        command.current_dir(cwd.as_path());
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start MCP server `{name}`"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("MCP server `{name}` has no stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("MCP server `{name}` has no stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("MCP server `{name}` has no stderr"))?;
    let stdin = Arc::new(Mutex::new(stdin));
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let closed = Arc::new(AtomicBool::new(false));
    spawn_stderr(stderr, tail.clone());
    spawn_reader(
        name.to_string(),
        stdout,
        stdin.clone(),
        pending.clone(),
        closed.clone(),
    );
    Ok(Server {
        name: name.to_string(),
        child,
        stdin,
        pending,
        stderr: tail,
        next_id: 1,
        timeout: Duration::from_secs(cfg.timeout_secs),
        closed,
        initialized: false,
    })
}

fn spawn_stderr(stderr: ChildStderr, tail: Arc<Mutex<VecDeque<String>>>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
            if tail.len() == 20 {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    });
}

fn spawn_reader(
    name: String,
    stdout: ChildStdout,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, Sender<Result<Value>>>>>,
    closed: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(_) => {
                    fail_all(&pending, &name, "sent malformed JSON");
                    closed.store(true, Ordering::SeqCst);
                    return;
                }
            };
            let id = value.get("id").and_then(Value::as_u64);
            let method = value.get("method").and_then(Value::as_str);
            match (id, method) {
                (Some(id), Some(_)) => {
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32601, "message": "method not supported"},
                    });
                    let _ = write_line(&stdin, &response);
                }
                (Some(id), None) => {
                    let sender = pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id);
                    let Some(sender) = sender else { continue };
                    if let Some(error) = value.get("error") {
                        let _ = sender.send(Err(anyhow!("MCP server `{name}` error: {error}")));
                    } else {
                        let result = value.get("result").cloned().unwrap_or(Value::Null);
                        let _ = sender.send(Ok(result));
                    }
                }
                (None, _) => {}
            }
        }
        fail_all(&pending, &name, "closed");
        closed.store(true, Ordering::SeqCst);
    });
}

fn fail_all(pending: &Arc<Mutex<HashMap<u64, Sender<Result<Value>>>>>, name: &str, reason: &str) {
    let mut pending = pending.lock().unwrap_or_else(|e| e.into_inner());
    for (_, sender) in pending.drain() {
        let _ = sender.send(Err(anyhow!("MCP server `{name}` {reason}")));
    }
}

fn write_line(stdin: &Arc<Mutex<ChildStdin>>, value: &Value) -> Result<()> {
    let mut stdin = stdin
        .lock()
        .map_err(|_| anyhow!("MCP server stdin is poisoned"))?;
    serde_json::to_writer(&mut *stdin, value)?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    Ok(())
}

impl Server {
    pub(crate) fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "mypi", "version": env!("CARGO_PKG_VERSION")},
        });
        self.request("initialize", params)?;
        self.notify("notifications/initialized", json!({}))?;
        self.initialized = true;
        Ok(())
    }

    pub(crate) fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        if self.is_closed() {
            bail!("MCP server `{}` is not running{}", self.name, self.tail());
        }
        let id = self.next_id;
        self.next_id += 1;
        let (sender, receiver) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| anyhow!("MCP server `{}` pending map is poisoned", self.name))?
            .insert(id, sender);
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        if let Err(error) = write_line(&self.stdin, &message) {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(error).with_context(|| format!("MCP server `{}` write failed", self.name));
        }
        match receiver.recv_timeout(self.timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                bail!(
                    "MCP server `{}` timed out after {}s on {method}{}",
                    self.name,
                    self.timeout.as_secs(),
                    self.tail()
                );
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!(
                    "MCP server `{}` closed during {method}{}",
                    self.name,
                    self.tail()
                );
            }
        }
    }

    pub(crate) fn notify(&self, method: &str, params: Value) -> Result<()> {
        if self.is_closed() {
            bail!("MCP server `{}` is not running{}", self.name, self.tail());
        }
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params});
        write_line(&self.stdin, &message)
            .with_context(|| format!("MCP server `{}` write failed", self.name))
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub(crate) fn shutdown(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        fail_all(&self.pending, &self.name, "shut down");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn tail(&self) -> String {
        let tail = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        if tail.is_empty() {
            return String::new();
        }
        let lines: Vec<&str> = tail.iter().map(String::as_str).collect();
        format!("\nstderr:\n{}", lines.join("\n"))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}
