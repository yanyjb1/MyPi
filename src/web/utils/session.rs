//! Browser session — the one place that owns attach-or-launch, tab
//! allocation, socket reconnection, and rendered-DOM polling for the whole
//! web domain.
//!
//! Everything above this file (search/fetch/browser `engine.rs` and their
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

pub(crate) const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const RENDER_WAIT: Duration = Duration::from_secs(15);
pub(crate) const POLL_TIMEOUT: Duration = Duration::from_secs(8);

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
    /// Named tab pool: `name → target id`. "work" is the browser tool's
    /// persistent page; other names can be pinned on demand. Tabs created
    /// by OTHER mypi processes sharing this Chromium are invisible here —
    /// by design, each process only owns its own pool.
    tabs: std::collections::HashMap<String, String>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

/// A live Chromium on `port` that some other mypi (or a human) started.
fn discover_live_browser(profile: &std::path::Path) -> Option<u16> {
    // DevToolsActivePort line 1 = port, written by Chromium at startup.
    // A file from a previous boot describes a dead port, so TCP-probe it.
    let first = std::fs::read_to_string(profile.join("DevToolsActivePort")).ok()?;
    let port = first.lines().next()?.trim().parse::<u16>().ok()?;
    Browser::port_alive(port).then_some(port)
}

fn ensure_browser() -> anyhow::Result<u16> {
    let mut guard = SESSION.lock();
    if let Some(s) = guard.as_ref() {
        if Browser::port_alive(s.browser.port) {
            return Ok(s.browser.port);
        }
        // Dead browser: drop it (kills our child if any) and respawn.
        *guard = None;
    }
    let profile = browser_profile_dir();
    // Rendezvous: another mypi in another directory may already run a
    // Chromium on this very profile (the default profile is shared via
    // XDG_DATA_HOME, which does not vary with cwd). Attaching keeps one
    // browser serving every session; launch is the fallback, not the norm.
    let browser = match std::env::var("MYPI_BROWSER_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .or_else(|| discover_live_browser(&profile))
    {
        Some(port) => Browser::attach(&profile, port).context("attaching to shared browser")?,
        None => Browser::launch(&profile).context("launching browser")?,
    };
    let port = browser.port;
    *guard = Some(Session { browser, tabs: Default::default() });
    Ok(port)
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
pub(crate) fn with_transient<T>(
    url: &str,
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let port = ensure_browser()?;
    let page = open_page(port, url)?;
    let result = f(&page);
    close_page(port, &page.target_id);
    result
}

/// Run `f` on a named tab from this process's pool, creating it (at `url`,
/// default about:blank) if absent. The pool is per-mypi-process — two
/// sessions sharing one Chromium each keep their own named tabs, and a
/// pooled tab survives between calls so consecutive tool invocations see
/// the same page. Stale entries (tab closed externally) are evicted and
/// re-created on the next call.
pub(crate) fn with_tab<T>(
    name: &str,
    url: Option<&str>,
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let port = ensure_browser()?;
    let page = {
        let mut guard = SESSION.lock();
        let Some(session) = guard.as_mut() else {
            anyhow::bail!("session vanished");
        };
        let browser = Browser::attach(&browser_profile_dir(), port)?;
        let live = browser.targets()?;
        match session.tabs.get(name).cloned() {
            Some(id) => match live.into_iter().find(|t| t.id == id) {
                Some(target) => connect_to(port, &target)?,
                // Tab was closed behind our back: evict and fall through
                // to re-creation below.
                None => {
                    session.tabs.remove(name);
                    recreate_tab(&mut session.tabs, name, port, url)?
                }
            },
            None => recreate_tab(&mut session.tabs, name, port, url)?,
        }
    };
    f(&page)
}

fn recreate_tab(
    tabs: &mut std::collections::HashMap<String, String>,
    name: &str,
    port: u16,
    url: Option<&str>,
) -> anyhow::Result<Page> {
    let browser = Browser::attach(&browser_profile_dir(), port)?;
    let target = browser.create_target(url.unwrap_or("about:blank"))?;
    tabs.insert(name.to_string(), target.id.clone());
    connect_to(port, &target)
}

/// The browser tool's persistent page: same tab across open/act/read.
pub(crate) fn with_work<T>(f: impl FnOnce(&Page) -> anyhow::Result<T>) -> anyhow::Result<T> {
    with_tab("work", None, f)
}
