//! The process-wide browser: when it exists, who owns it, and how it ends.

mod idle;
#[cfg(unix)]
mod signal;
mod timer;

pub(crate) use idle::touch;

use parking_lot::Mutex;
use std::time::Duration;

use super::process::{self, Browser};
use super::tabs;
use crate::server::config::Config;

struct Live {
    browser: Browser,
    idle: Duration,
    bring_to_front: bool,
}

static STATE: Mutex<Option<Live>> = Mutex::new(None);

/// The port to talk to, launching the browser if there is none.
pub(crate) fn port() -> anyhow::Result<u16> {
    let mut guard = STATE.lock();
    if let Some(live) = guard.as_mut()
        && live.browser.alive()
    {
        return Ok(live.browser.port());
    }
    let dead = guard.take();
    drop(dead);
    let cfg = Config::load()?;
    let opts = cfg.browser();
    let profile = process::profile_dir(&opts);
    let browser = match std::env::var("MYPI_BROWSER_PORT")
        .ok()
        .and_then(|port| port.parse::<u16>().ok())
        .or(opts.port)
        .or_else(|| process::discover_live_browser(&profile))
    {
        Some(port) => Browser::attach(port)?,
        None => Browser::launch(&opts, &profile)?,
    };
    let port = browser.port();
    *guard = Some(Live {
        browser,
        idle: opts.idle,
        bring_to_front: cfg.bring_to_front(),
    });
    drop(guard);
    idle::touch();
    timer::start();
    #[cfg(unix)]
    signal::install();
    Ok(port)
}

/// The port of a browser that already exists; never launches one.
pub(crate) fn existing_port() -> Option<u16> {
    let mut guard = STATE.lock();
    let live = guard.as_mut()?;
    live.browser.alive().then(|| live.browser.port())
}

/// Whether the browser is this process's to close.
pub(crate) fn owned() -> bool {
    STATE
        .lock()
        .as_ref()
        .is_some_and(|live| !live.browser.attached())
}

/// Whether a verification challenge should raise the window.
pub(crate) fn bring_to_front() -> bool {
    STATE
        .lock()
        .as_ref()
        .is_some_and(|live| live.bring_to_front)
}

/// Close the browser and forget every tab. Idempotent.
pub fn shutdown() {
    let dead = STATE.lock().take();
    drop(dead);
    tabs::clear();
}
