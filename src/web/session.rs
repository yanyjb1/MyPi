//! Browser session — the one place that owns attach-or-launch, tab
//! allocation, socket reconnection, and rendered-DOM polling for the whole
//! web domain.
//!
//! Everything above this file (`ddg`, `fetch`, the `browser` tool) talks to
//! a `Page`; nothing else imports `cdp` directly.
//!
//! Two access patterns, both per-call and page-pinned:
//!
//! - [`with_transient`] — search/fetch: a throwaway tab, opened for the
//!   call, closed after. Their navigations can never disturb anyone.
//! - [`with_work`] — the browser tool: one persistent *work tab* for the
//!   process, so `open` → `act` → `read` → `screenshot` all land on the
//!   same page the user is watching.
//!
//! The browser process itself lives for the process (attach to
//! `MYPI_BROWSER_PORT`, or spawn headless Helium and keep the child
//! handle — dropping it would kill the browser mid-session).

use anyhow::{Context as _, anyhow};
use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::cdp::{Browser, Cdp, Target};
use crate::xdg::browser_profile_dir;

pub(super) const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(20);
pub(super) const RENDER_WAIT: Duration = Duration::from_secs(15);
pub(super) const POLL_TIMEOUT: Duration = Duration::from_secs(8);

/// One connected page, pinned to a specific tab. Clones share the socket
/// behind a mutex; navigation resets that socket and the next call heals
/// it **on the same tab** (reconnection re-resolves by target id).
#[derive(Clone)]
pub struct Page {
    port: u16,
    target_id: String,
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
    /// On budget exhaustion this is *best-effort success*: the last HTML
    /// snapshot is returned, not an error. Callers that need strict
    /// failure (DDG's no-results case) express it through their predicate.
    pub fn wait(
        &self,
        budget: Duration,
        ready: impl Fn(&str) -> Option<anyhow::Result<()>>,
    ) -> anyhow::Result<String> {
        let deadline = Instant::now() + budget;
        let mut last = String::new();
        loop {
            match self.dom_html() {
                Ok(html) => {
                    match ready(&html) {
                        Some(Ok(())) => return Ok(html),
                        Some(Err(e)) => return Err(e),
                        None => last = html,
                    }
                }
                // Navigation reset the socket; keep polling — the next
                // `resilient` call re-resolves the same tab transparently.
                Err(e) if is_reset(&e) => {}
                Err(e) => return Err(e),
            }
            if Instant::now() > deadline {
                return Ok(last); // render what we have rather than fail
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
        super::url::base64_decode(&data)
    }

    /// Devtools HTTP port (for diagnostics output).
    pub fn port(&self) -> u16 {
        self.port
    }

    // One CDP command with reconnect-and-retry on a reset socket. The
    // replacement socket is bound to the SAME target id — not "the first
    // page in /json/list" — so a healed connection still points at this
    // page even if other tabs were opened meanwhile.
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
        let target = browser
            .targets()?
            .into_iter()
            .find(|t| t.id == self.target_id)
            .ok_or_else(|| anyhow!("tab {} vanished", self.target_id))?;
        Cdp::connect(&target.ws_url)
    }
}

fn is_reset(e: &anyhow::Error) -> bool {
    e.to_string().contains("connection reset")
}

// --- Process-wide browser ----------------------------------------------------

struct Session {
    browser: Browser, // owns the child when we spawned it; Drop kills
    work_tab: Option<String>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

fn ensure_browser() -> anyhow::Result<(u16, ())> {
    let mut guard = SESSION.lock();
    if let Some(s) = guard.as_ref() {
        if Browser::port_alive(s.browser.port) {
            return Ok((s.browser.port, ()));
        }
        // Dead browser: drop it (kills our child if any) and respawn.
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
    *guard = Some(Session { browser, work_tab: None });
    Ok((port, ()))
}

fn connect_to(port: u16, target: &Target) -> anyhow::Result<Page> {
    let cdp = Cdp::connect(&target.ws_url)?;
    Ok(Page {
        port,
        target_id: target.id.clone(),
        cdp: Arc::new(Mutex::new(cdp)),
    })
}

fn open_page(port: u16, url: &str) -> anyhow::Result<Page> {
    let browser = Browser::attach(&browser_profile_dir(), port)?;
    let target = browser.create_target(url)?;
    connect_to(port, &target)
}

fn close_page(port: u16, target_id: &str) {
    // Best-effort: a vanished tab is fine (the call is over anyway).
    if let Ok(browser) = Browser::attach(&browser_profile_dir(), port) {
        let _ = browser.close_target(target_id);
    }
}

/// Run `f` on a throwaway tab; the tab is closed afterwards. For search /
/// fetch — page-level state (cookies, uBlock) lives in the shared profile,
/// so a fresh tab keeps all of it and costs only one createTarget round
/// trip.
pub(super) fn with_transient<T>(
    url: &str,
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let (port, ()) = ensure_browser()?;
    let page = open_page(port, url)?;
    let result = f(&page);
    close_page(port, &page.target_id);
    result
}

/// Run `f` on the process-wide *work tab* (lazily created at about:blank).
/// The browser tool uses this so consecutive `open`/`act`/`read` calls
/// address the same page.
pub(super) fn with_work<T>(f: impl FnOnce(&Page) -> anyhow::Result<T>) -> anyhow::Result<T> {
    let (port, ()) = ensure_browser()?;
    let (page, fresh) = {
        let mut guard = SESSION.lock();
        let Some(session) = guard.as_mut() else {
            anyhow::bail!("session vanished");
        };
        match session.work_tab.clone() {
            Some(id) => {
                // Re-pin the existing work tab (survives tab list churn).
                let browser = Browser::attach(&browser_profile_dir(), port)?;
                let target = browser
                    .targets()?
                    .into_iter()
                    .find(|t| t.id == id)
                    .ok_or_else(|| anyhow!("work tab vanished"))?;
                (connect_to(port, &target)?, false)
            }
            None => {
                let browser = Browser::attach(&browser_profile_dir(), port)?;
                let target = browser.create_target("about:blank")?;
                session.work_tab = Some(target.id.clone());
                (connect_to(port, &target)?, true)
            }
        }
    };
    let _ = fresh;
    f(&page)
}
