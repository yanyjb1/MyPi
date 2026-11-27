// The CDP WebSocket transport: connect, send, receive.

use anyhow::{Context as _, anyhow};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
use tungstenite::Message;

use crate::browser::halt;

// The two ids no request reaches: one ends the wait, the other says the socket is gone.
const ABORT: u64 = u64::MAX;
const RESET: u64 = u64::MAX - 1;

type Inbox = std::sync::mpsc::Sender<(u64, Value)>;

pub(crate) struct Cdp {
    next_id: u64,
    inbox: Receiver<(u64, Value)>,
    outbox: std::sync::mpsc::Sender<Message>,
    knocker: Arc<Knocker>,
    _pump: std::thread::JoinHandle<()>,
}

// One page's stop: the flag every connection of it reads, and the wire of the one that is live now.
pub(crate) struct Knocker {
    woken: AtomicBool,
    wake: Mutex<Option<Inbox>>,
}

impl Knocker {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            woken: AtomicBool::new(false),
            wake: Mutex::new(None),
        })
    }

    // Each connection parks its inbox here, so a stop reaches the wait that is holding the wire now.
    fn attach(&self, wake: Inbox) {
        *self.wake.lock() = Some(wake);
    }

    // End the call waiting on this page, and every call that would follow, on any connection.
    pub(crate) fn abort(&self) {
        self.woken.store(true, Ordering::Release);
        if let Some(wake) = self.wake.lock().clone() {
            let _ = wake.send((ABORT, Value::Null));
        }
    }

    pub(crate) fn aborted(&self) -> bool {
        self.woken.load(Ordering::Acquire)
    }
}

impl Cdp {
    // `knocker` belongs to the page: a stop that cut one connection also ends the one that replaces it.
    pub(crate) fn connect(ws_url: &str, knocker: Arc<Knocker>) -> anyhow::Result<Cdp> {
        let (mut ws, _resp) =
            tungstenite::connect(ws_url).with_context(|| format!("connecting {ws_url}"))?;

        if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
            s.set_read_timeout(Some(Duration::from_millis(50)))?;
        }

        let (in_tx, in_rx) = channel::<(u64, Value)>();
        let (out_tx, out_rx) = channel::<Message>();
        knocker.attach(in_tx.clone());

        // The pump owns the socket; its 50 ms read timeout only decides how often it looks at the outbox.
        let heard = in_tx;
        let pump = std::thread::spawn(move || {
            'pump: loop {
                while let Ok(msg) = out_rx.try_recv() {
                    if ws.send(msg).is_err() {
                        break 'pump;
                    }
                }
                let frame = match ws.read() {
                    Ok(Message::Text(text)) => text,
                    Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_)) => continue,
                    Err(tungstenite::Error::Io(e))
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        continue;
                    }
                    Ok(Message::Close(_)) | Err(_) => break 'pump,
                    Ok(_) => continue,
                };
                if let Ok(v) = serde_json::from_str::<Value>(&frame) {
                    let id = v.get("id").and_then(Value::as_u64).unwrap_or(0);
                    if heard.send((id, v)).is_err() {
                        break 'pump;
                    }
                }
            }
            // The socket is gone: the inbox outlives it as long as the page holds a handle, so the death is said here.
            let _ = heard.send((RESET, Value::Null));
        });

        Ok(Cdp {
            next_id: 0,
            inbox: in_rx,
            outbox: out_tx,
            knocker,
            _pump: pump,
        })
    }

    pub(crate) fn call(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        if self.knocker.aborted() {
            return Err(halt::Stopped.into());
        }
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({"id": id, "method": method, "params": params});
        self.outbox
            .send(Message::text(msg.to_string()))
            .map_err(|_| anyhow!("{method}: connection reset (navigation or page closed)"))?;

        let deadline = Instant::now() + timeout;
        loop {
            let (got_id, v) = self
                .inbox
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|e| match e {
                    std::sync::mpsc::RecvTimeoutError::Disconnected => {
                        anyhow!("{method}: connection reset (navigation or page closed)")
                    }
                    std::sync::mpsc::RecvTimeoutError::Timeout => {
                        anyhow!("timeout waiting for {method} response")
                    }
                })?;
            if got_id == ABORT {
                return Err(halt::Stopped.into());
            }
            // The socket is gone: the page closed, or it was reattached under us.
            if got_id == RESET {
                return Err(anyhow!(
                    "{method}: connection reset (navigation or page closed)"
                ));
            }
            if got_id == id {
                if let Some(err) = v.get("error") {
                    return Err(anyhow!("{method} failed: {err}"));
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    pub(crate) fn navigate(&mut self, url: &str, timeout: Duration) -> anyhow::Result<()> {
        self.call("Page.enable", json!({}), Duration::from_secs(5))
            .ok();
        if self.knocker.aborted() {
            return Err(halt::Stopped.into());
        }
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({"id": id, "method": "Page.navigate", "params": {"url": url}});
        self.outbox
            .send(Message::text(msg.to_string()))
            .context("ws send")?;

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let (got_id, v) = match self
                .inbox
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(pair) => pair,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
            };
            if got_id == ABORT {
                return Err(halt::Stopped.into());
            }
            if got_id == RESET {
                return Ok(());
            }
            if got_id == id {
                if let Some(err) = v.get("error") {
                    return Err(anyhow!("Page.navigate failed: {err}"));
                }
                return Ok(());
            }
            if v.get("method").and_then(Value::as_str) == Some("Page.loadEventFired") {
                return Ok(());
            }
        }
        anyhow::bail!("timeout navigating to {url}")
    }

    pub(crate) fn evaluate(
        &mut self,
        expression: &str,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        let r = self.call(
            "Runtime.evaluate",
            json!({"expression": expression, "returnByValue": true}),
            timeout,
        )?;
        Ok(r.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }

    pub(crate) fn dom_html(&mut self, timeout: Duration) -> anyhow::Result<String> {
        for attempt in 0..2 {
            let doc = self.call("DOM.getDocument", json!({}), timeout)?;
            let root = doc
                .pointer("/root/nodeId")
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow!("DOM.getDocument: no root"))?;
            match self.call("DOM.getOuterHTML", json!({"nodeId": root}), timeout) {
                Ok(html) => {
                    return Ok(html
                        .get("outerHTML")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string());
                }
                Err(e) if attempt == 0 && e.to_string().contains("Could not find node") => {
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!("retry loop returns on both arms")
    }
}
