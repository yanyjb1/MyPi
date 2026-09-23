//! Browser session — the one place that owns attach-or-launch, socket
//! reconnection, and rendered-DOM polling for the whole web domain.
//!
//! Everything above this file (`ddg`, `fetch`, `browser` tool) talks to a
//! `Page`; nothing else imports `cdp` directly. The session lives for the
//! process: the first call attaches to `MYPI_BROWSER_PORT` (a browser
//! someone keeps running — session warmth matters to anti-bot frontends)
//! or spawns a headless Helium, and later calls reuse it after a cheap
//! liveness probe.

use anyhow::{Context as _, anyhow};
use serde_json::json;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::cdp::{Browser, Cdp};
use crate::xdg::browser_profile_dir;

pub(super) const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(20);
pub(super) const RENDER_WAIT: Duration = Duration::from_secs(15);
pub(super) const POLL_TIMEOUT: Duration = Duration::from_secs(8);

/// One connected page. Clones share the same underlying socket behind a
/// mutex; navigation resets that socket and the next call heals it.
#[derive(Clone)]
pub struct Page {
    port: u16,
    cdp: Arc<Mutex<Cdp>>,
}

impl Page {
    /// Navigate; the ack is enough (cross-document navigation tears the
    /// per-target socket down, so waiting for loadEventFired here would
    /// race the disconnect). Readiness is the caller's `wait` predicate.
    pub fn navigate(&self, url: &str) -> anyhow::Result<()> {
        self.resilient(|c| c.navigate(url, NAVIGATE_TIMEOUT).map(|_| ()))
    }

    /// `Runtime.evaluate` with `returnByValue`. Best-effort: bot-hardened
    /// pages can hang this channel (live-verified) — prefer `dom_html`
    /// for reads.
    pub fn evaluate(&self, expression: &str, timeout: Duration) -> anyhow::Result<String> {
        self.resilient(|c| {
            let v = c.evaluate(expression, timeout)?;
            Ok(match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            })
        })
    }

    /// Current DOM through the DOM domain — the bot-proof read path.
    pub fn dom_html(&self) -> anyhow::Result<String> {
        self.resilient(|c| c.dom_html(POLL_TIMEOUT))
    }

    /// Poll `dom_html` until `ready` decides. `ready` returning:
    /// - `None` → keep polling (page not there yet)
    /// - `Some(Ok(()))` → satisfied; returns the current HTML
    /// - `Some(Err(e))` → abort (anomaly wall etc.)
    ///
    /// Socket resets inside the poll window are healed transparently —
    /// the tab id survives cross-document navigation even when the WS
    /// does not. The satisfied page HTML is the return value; callers
    /// parse it themselves (parsers are pure functions).
    pub fn wait(
        &self,
        budget: Duration,
        ready: impl Fn(&str) -> Option<anyhow::Result<()>>,
    ) -> anyhow::Result<String> {
        let deadline = Instant::now() + budget;
        loop {
            match self.dom_html() {
                Ok(html) => match ready(&html) {
                    Some(Ok(())) => return Ok(html),
                    Some(Err(e)) => return Err(e),
                    None => {}
                },
                // Navigation reset the socket; keep polling — the next
                // `resilient` call re-resolves the target transparently.
                Err(e) if is_reset(&e) => {}
                Err(e) => return Err(e),
            }
            if Instant::now() > deadline {
                anyhow::bail!("browser page never became ready within {budget:?}");
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    }

    /// PNG screenshot of the current viewport (or full page).
    pub fn screenshot(&self, full_page: bool) -> anyhow::Result<Vec<u8>> {
        let mut params = json!({"format": "png"});
        if full_page {
            params["captureBeyondViewport"] = json!(true);
        }
        let data = self.resilient(|c| {
            let r = c.call("Page.captureScreenshot", params.clone(), Duration::from_secs(15))?;
            r.get("data")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow!("no screenshot data"))
        })?;
        crate::web::url::base64_decode(&data)
    }

    /// Devtools HTTP port (for diagnostics output).
    pub fn port(&self) -> u16 {
        self.port
    }

    // One CDP command with a single reconnect-and-retry on a reset socket.
    fn resilient<T>(&self, f: impl Fn(&mut Cdp) -> anyhow::Result<T>) -> anyhow::Result<T> {
        {
            let mut guard = self.cdp.lock();
            match f(&mut guard) {
                Ok(v) => return Ok(v),
                Err(e) if is_reset(&e) => {} // heal below
                Err(e) => return Err(e),
            }
        }
        let mut guard = self.cdp.lock();
        *guard = self.connect_fresh()?;
        f(&mut guard)
    }

    fn connect_fresh(&self) -> anyhow::Result<Cdp> {
        let browser = Browser::attach(&browser_profile_dir(), self.port)
            .context("re-attaching to browser")?;
        let target = browser.page_target().context("no page target")?;
        Cdp::connect(&target.ws_url)
    }
}

fn is_reset(e: &anyhow::Error) -> bool {
    e.to_string().contains("connection reset")
}

// --- Process-wide session ----------------------------------------------------
//
// The Mutex<Option<..>> guards the agent loop's concurrent tool calls; the
// Option stays None until the first call needs a browser. Tests inject via
// `with_page_for` instead of touching this.

struct Live {
    port: u16,
    cdp: Arc<Mutex<Cdp>>,
}

static SESSION: Mutex<Option<Live>> = Mutex::new(None);

fn acquire() -> anyhow::Result<Page> {
    let mut guard = SESSION.lock();
    if let Some(live) = guard.as_ref() {
        if Browser::port_alive(live.port) {
            return Ok(Page { port: live.port, cdp: Arc::clone(&live.cdp) });
        }
        *guard = None;
    }
    let profile = browser_profile_dir();
    let browser = match std::env::var("MYPI_BROWSER_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
    {
        Some(port) => Browser::attach(&profile, port).context("attaching to MYPI_BROWSER_PORT")?,
        None => Browser::launch(&profile).context("launching browser")?,
    };
    let port = browser.port;
    let target = browser.page_target().context("no page target")?;
    let cdp = Arc::new(Mutex::new(Cdp::connect(&target.ws_url)?));
    *guard = Some(Live { port, cdp: Arc::clone(&cdp) });
    Ok(Page { port, cdp })
}

/// Run `f` against the shared session's page.
pub(super) fn with_page<T>(
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    f(&acquire()?)
}
