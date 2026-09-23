//! CDP (Chrome DevTools Protocol) client — the WebSocket layer plus the
//! smallest request/response core the tools need.
//!
//! Scope, deliberately narrow: launch-or-connect to a Chromium-family
//! browser (Helium included), open tabs, navigate, evaluate, and pull the
//! rendered DOM. Everything else (clicking, typing, screenshots) builds on
//! the same `Cdp::call` primitive later.
//!
//! Protocol facts verified live against Helium 0.17 (Chromium 153):
//!  • `--remote-debugging-port=0` writes `<profile>/DevToolsActivePort`
//!    (line 1: port, line 2: browser WS path).
//!  • Requests carry a caller-chosen `id`; responses echo it. Events have
//!    no `id` and interleave with responses — a reader thread must drain
//!    the socket continuously or pending responses starve behind events.
//!  • Per-target connections (`/devtools/page/<id>`) skip session
//!    multiplexing entirely; that's what we use.
use anyhow::{Context as _, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
use tungstenite::Message;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::WebSocket;

type WsStream = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

// A browser handle: either a process we spawned (owned, killed on drop) or
// one already running (attached; the owner keeps the lifecycle).
pub struct Browser {
    child: Option<Child>,
    profile: PathBuf,
    pub port: u16,
}

impl Browser {
    /// Launch headless Helium (or any Chromium-family binary) with a fresh
    /// debugging port. The profile is **persistent** (see
    /// [`crate::xdg::browser_profile_dir`]) so logins/extensions survive
    /// restarts.
    pub fn launch(profile: &Path) -> anyhow::Result<Browser> {
        let exe = std::env::var_os("MYPI_BROWSER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/opt/helium/helium"));
        Self::launch_with(&exe, profile, &[])
    }

    /// Attach to an already-running browser on a fixed debugging port.
    /// The long-lived-session mode: one browser stays up, every tool call
    /// reuses it — cheaper than spawn-per-call and far friendlier to
    /// anti-bot frontends, which weigh session age and cookie warmth.
    pub fn attach(profile: &Path, port: u16) -> anyhow::Result<Browser> {
        if !Self::port_alive(port) {
            anyhow::bail!("no browser listening on 127.0.0.1:{port}");
        }
        Ok(Browser { child: None, profile: profile.to_path_buf(), port })
    }

    pub fn launch_with(exe: &Path, profile: &Path, extra_args: &[&str]) -> anyhow::Result<Browser> {
        std::fs::create_dir_all(profile)
            .with_context(|| format!("creating browser profile dir {}", profile.display()))?;

        let mut cmd = Command::new(exe);
        cmd.arg(format!("--user-data-dir={}", profile.display()))
            .arg("--remote-debugging-port=0")
            // Loopback only: never expose the debugger past the machine.
            .arg("--remote-debugging-address=127.0.0.1")
            .args(["--no-first-run", "--no-default-browser-check"])
            .arg("--headless=new")
            // HeadlessChrome in the UA is the loudest automation tell; the
            // stable channel version keeps the engine segment truthful.
            .arg("--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36")
            .args(extra_args)
            .arg("about:blank")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = cmd
            .spawn()
            .with_context(|| format!("spawning browser {}", exe.display()))?;

        let browser = Browser { child: Some(child), profile: profile.to_path_buf(), port: 0 };
        browser.wait_for_devtools()
    }


    fn wait_for_devtools(mut self) -> anyhow::Result<Browser> {
        // DevToolsActivePort appears once the WS endpoint is listening; the
        // child exiting early means the binary is wrong or the profile is
        // locked (second browser on the same dir).
        let port_file = self.profile.join("DevToolsActivePort");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(first) = std::fs::read_to_string(&port_file) {
                if let Some(line) = first.lines().next() {
                    if let Ok(port) = line.trim().parse::<u16>() {
                        // A stale file from a previous session describes a
                        // dead port — the file exists the instant the profile
                        // does. Only accept it once /json/version answers.
                        if Self::port_alive(port) {
                            self.port = port;
                            return Ok(self);
                        }
                    }
                }
            }
            if self
                .child
                .as_mut()
                .map(|c| c.try_wait().map(|s| s.is_some()))
                .transpose()?
                == Some(true)
            {
                anyhow::bail!("browser exited before opening DevTools port");
            }
            if Instant::now() > deadline {
                anyhow::bail!("timeout waiting for DevToolsActivePort");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// True when something accepts TCP on 127.0.0.1:port.
    fn port_alive(port: u16) -> bool {
        std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_millis(500),
        )
        .is_ok()
    }

    /// GET /json/list, parsed. Works against any Chromium.
    pub fn targets(&self) -> anyhow::Result<Vec<Target>> {
        let url = format!("http://127.0.0.1:{}/json/list", self.port);
        let body: Vec<Target> = ureq::get(&url)
            .header("Host", &format!("127.0.0.1:{}", self.port))
            .call()
            .map_err(|e| anyhow!("json/list failed: {e}"))?
            .body_mut()
            .read_json()
            .context("parsing /json/list")?;
        Ok(body)
    }

    /// The single `page` target — the tools only ever need one tab; multiple
    /// tabs are a later concern.
    pub fn page_target(&self) -> anyhow::Result<Target> {
        self.targets()?
            .into_iter()
            .find(|t| t.r#type == "page")
            .ok_or_else(|| anyhow!("no page target"))
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // Attached browsers are not ours to kill — only the process we
        // spawned gets reaped.
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Target {
    #[serde(rename = "type")]
    pub r#type: String,
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(rename = "webSocketDebuggerUrl", default)]
    pub ws_url: String,
}

// --- WebSocket session -------------------------------------------------------

pub struct Cdp {
    next_id: u64,
    inbox: Receiver<(u64, Value)>,
    outbox: std::sync::mpsc::Sender<Message>,
    _pump: std::thread::JoinHandle<()>,
}

impl Cdp {
    /// Connect to an already-listening page target.
    ///
    /// One pump thread owns the socket, multiplexing reads and writes
    /// through channels. (The earlier mutex+reader design starved the
    /// writer: the reader re-acquired the lock faster than `call` could.)
    /// A 50 ms read budget makes the pump loop periodic — between reads it
    /// drains the outgoing queue — so a quiet socket never parks outgoing
    /// traffic.
    pub fn connect(ws_url: &str) -> anyhow::Result<Cdp> {
        let (mut ws, _resp) = tungstenite::connect(ws_url)
            .with_context(|| format!("connecting {ws_url}"))?;

        if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
            s.set_read_timeout(Some(Duration::from_millis(50)))?;
        }

        let (in_tx, in_rx) = channel::<(u64, Value)>();
        let (out_tx, out_rx) = channel::<Message>();

        let pump = std::thread::spawn(move || loop {
            // 1) speak everything queued (cheap, non-blocking)
            while let Ok(msg) = out_rx.try_recv() {
                if ws.send(msg).is_err() {
                    return; // socket closed
                }
            }
            // 2) listen within the read budget
            let frame = match ws.read() {
                Ok(Message::Text(text)) => text,
                Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_)) => continue,
                // The 50 ms read budget expires as WouldBlock — that is the
                // loop's heartbeat, not a failure. Only real closures end
                // the pump.
                Err(tungstenite::Error::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Ok(Message::Close(_)) | Err(_) => return, // socket closed
                Ok(_) => continue,
            };
            // 3) forward inbound
            if let Ok(v) = serde_json::from_str::<Value>(&frame) {
                let id = v.get("id").and_then(Value::as_u64).unwrap_or(0);
                if in_tx.send((id, v)).is_err() {
                    return; // receiver gone: session closed
                }
            }
        });

        Ok(Cdp {
            next_id: 0,
            inbox: in_rx,
            outbox: out_tx,
            _pump: pump,
        })
    }

    /// Send a command and wait for its response (events skipped en route).
    /// Timeout is global per call; CDP has no per-command deadline of its own.
    pub fn call(&mut self, method: &str, params: Value, timeout: Duration) -> anyhow::Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({"id": id, "method": method, "params": params});
        self.outbox
            .send(Message::text(msg.to_string()))
            .map_err(|_| {
                // Pump gone = socket torn down (navigation reset or page
                // closed). Same recovery path as a dead recv: caller
                // reconnects.
                anyhow!("{method}: connection reset (navigation or page closed)")
            })?;

        let deadline = Instant::now() + timeout;
        loop {
            let (got_id, v) = self.inbox.recv_timeout(
                deadline.saturating_duration_since(Instant::now()),
            ).map_err(|e| match e {
                std::sync::mpsc::RecvTimeoutError::Disconnected => {
                    anyhow!("{method}: connection reset (navigation or page closed)")
                }
                std::sync::mpsc::RecvTimeoutError::Timeout => {
                    anyhow!("timeout waiting for {method} response")
                }
            })?;
            if got_id == id {
                if let Some(err) = v.get("error") {
                    return Err(anyhow!("{method} failed: {err}"));
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
            // An event or a stale response: keep draining.
        }
    }

    /// Navigate the connected page; returns once the load event fired or
    /// `timeout` elapsed. The navigate ack and the load event are consumed
    /// in ONE loop: either may arrive first, and an ack-only return is fine
    /// (callers poll content with their own deadline anyway).
    pub fn navigate(&mut self, url: &str, timeout: Duration) -> anyhow::Result<()> {
        self.call("Page.enable", json!({}), Duration::from_secs(5)).ok();
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({"id": id, "method": "Page.navigate", "params": {"url": url}});
        self.outbox
            .send(Message::text(msg.to_string()))
            .context("ws send")?;

        let deadline = Instant::now() + timeout;
        let mut acked = false;
        while Instant::now() < deadline {
            // A cross-document navigation can make Chromium tear the
            // per-target socket down; the pump exits and this channel
            // closes. That's not a failure of the navigation — treat it as
            // "in flight", return, and let the caller reconnect + poll.
            let (got_id, v) = match self
                .inbox
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(pair) => pair,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
            };
            if got_id == id {
                if let Some(err) = v.get("error") {
                    return Err(anyhow!("Page.navigate failed: {err}"));
                }
                // Ack in hand: navigation accepted. The load event may be
                // eaten by the socket reset that cross-document navigation
                // itself causes, so don't wait for it here — the caller's
                // content poll is the authoritative readiness check.
                return Ok(());
            }
            if v.get("method").and_then(Value::as_str) == Some("Page.loadEventFired") {
                return Ok(());
            }
        }
        // Acked but no load event: the socket reset ate it. Don't burn the
        // full budget here — content polling is the real readiness signal.
        if acked {
            return Ok(());
        }
        anyhow::bail!("timeout navigating to {url}")
    }

    /// `Runtime.evaluate` with `returnByValue`; scalar/string results only.
    /// NOTE (live-verified): on some bot-hardened pages evaluate hangs the
    /// renderer — `dom_html` is the reliable read path; evaluate is for
    /// interaction primitives.
    pub fn evaluate(&mut self, expression: &str, timeout: Duration) -> anyhow::Result<Value> {
        let r = self.call(
            "Runtime.evaluate",
            json!({"expression": expression, "returnByValue": true}),
            timeout,
        )?;
        Ok(r.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }

    /// `DOM.getDocument` + `DOM.getOuterHTML` — the bot-proof read path.
    /// Verified live: works where `Runtime.evaluate` hangs.
    pub fn dom_html(&mut self, timeout: Duration) -> anyhow::Result<String> {
        let doc = self.call("DOM.getDocument", json!({}), timeout)?;
        let root = doc
            .pointer("/root/nodeId")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("DOM.getDocument: no root"))?;
        let html = self.call(
            "DOM.getOuterHTML",
            json!({"nodeId": root}),
            timeout,
        )?;
        Ok(html
            .get("outerHTML")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }
}
