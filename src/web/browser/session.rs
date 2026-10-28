use anyhow::{anyhow, bail};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::page::{self, Page};
use super::process::{self, Browser};
use super::reaper;
use crate::server::config::BrowserConfig;

const IDLE: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TabKind {
    Search,
    Work,
}

struct Tab {
    target_id: String,
    last_used: Instant,
    active: usize,
    reserved: bool,
}

struct State {
    browser: Browser,
    tabs: HashMap<(i64, TabKind), Tab>,
    challenges: HashSet<(i64, String)>,
    search_locks: HashMap<i64, Arc<Mutex<()>>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn carry(mut state: Option<State>) -> (HashSet<(i64, String)>, HashMap<i64, Arc<Mutex<()>>>) {
    match state.take() {
        Some(s) => (s.challenges, s.search_locks),
        None => (HashSet::new(), HashMap::new()),
    }
}

fn ensure_browser(cfg: &BrowserConfig, headless: bool) -> anyhow::Result<u16> {
    let mut guard = STATE.lock();
    if let Some(state) = guard.as_ref() {
        if Browser::port_alive(state.browser.port) {
            if state.browser.attached() {
                return Ok(state.browser.port);
            }
            if headless || !state.browser.headless() {
                return Ok(state.browser.port);
            }
        }
    }
    let (challenges, search_locks) = carry(guard.take());
    let profile = process::profile_dir(cfg);
    let browser = match std::env::var("MYPI_BROWSER_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .or(cfg.port)
        .or_else(|| process::discover_live_browser(&profile))
    {
        Some(port) => Browser::attach(&profile, port)?,
        None => Browser::launch(cfg, &profile, headless)?,
    };
    let port = browser.port;
    *guard = Some(State {
        browser,
        tabs: HashMap::new(),
        challenges,
        search_locks,
    });
    drop(guard);
    reaper::start();
    Ok(port)
}

pub(crate) fn ensure_headed(cfg: &BrowserConfig) -> anyhow::Result<()> {
    if !has_display() {
        bail!("no display is available for a headed browser");
    }
    let mut guard = STATE.lock();
    if let Some(state) = guard.as_ref() {
        if state.browser.attached() {
            bail!("attached browser cannot switch to headed");
        }
        if !state.browser.headless() {
            return Ok(());
        }
    }
    let (challenges, search_locks) = carry(guard.take());
    let profile = process::profile_dir(cfg);
    let browser = Browser::launch(cfg, &profile, false)?;
    *guard = Some(State {
        browser,
        tabs: HashMap::new(),
        challenges,
        search_locks,
    });
    drop(guard);
    reaper::start();
    Ok(())
}

pub(crate) fn has_display() -> bool {
    if cfg!(target_os = "linux") {
        std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
    } else {
        true
    }
}

struct Lease {
    key: (i64, TabKind),
    page: Page,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut guard = STATE.lock();
        if let Some(state) = guard.as_mut()
            && let Some(tab) = state.tabs.get_mut(&self.key)
        {
            tab.active = tab.active.saturating_sub(1);
            tab.last_used = Instant::now();
            if self.key.1 == TabKind::Work {
                tab.reserved = false;
            }
        }
    }
}

fn with_tab<T>(
    port: u16,
    session: i64,
    kind: TabKind,
    cfg: &BrowserConfig,
    reserve: bool,
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let page = {
        let mut guard = STATE.lock();
        let state = guard
            .as_mut()
            .ok_or_else(|| anyhow!("browser state vanished"))?;
        let browser = Browser::attach(&process::profile_dir(cfg), port)?;
        let live = browser.targets()?;
        let key = (session, kind);
        let target = match state
            .tabs
            .get(&key)
            .and_then(|tab| live.iter().find(|t| t.id == tab.target_id).cloned())
        {
            Some(target) => target,
            None => {
                state.tabs.remove(&key);
                if kind == TabKind::Search {
                    state.challenges.retain(|(s, _)| *s != session);
                }
                let target = browser.create_target("about:blank")?;
                state.tabs.insert(
                    key,
                    Tab {
                        target_id: target.id.clone(),
                        last_used: Instant::now(),
                        active: 0,
                        reserved: false,
                    },
                );
                target
            }
        };
        let page = page::connect(port, &target)?;
        let tab = state
            .tabs
            .get_mut(&key)
            .ok_or_else(|| anyhow!("tab vanished"))?;
        tab.active += 1;
        tab.last_used = Instant::now();
        if kind == TabKind::Work {
            tab.reserved = reserve;
        }
        page
    };
    let lease = Lease {
        key: (session, kind),
        page,
    };
    f(&lease.page)
}

pub(crate) fn with_search<T>(
    session: i64,
    cfg: &BrowserConfig,
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let port = ensure_browser(cfg, cfg.headless)?;
    let lock = {
        let mut guard = STATE.lock();
        let state = guard
            .as_mut()
            .ok_or_else(|| anyhow!("browser state vanished"))?;
        state
            .search_locks
            .entry(session)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _serial = lock.lock();
    with_tab(port, session, TabKind::Search, cfg, false, f)
}

pub(crate) fn with_work<T>(
    session: i64,
    cfg: &BrowserConfig,
    reserve: bool,
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let port = ensure_browser(cfg, cfg.headless)?;
    with_tab(port, session, TabKind::Work, cfg, reserve, f)
}

pub fn close_session(session: i64) {
    let mut guard = STATE.lock();
    let Some(state) = guard.as_mut() else {
        return;
    };
    for kind in [TabKind::Search, TabKind::Work] {
        if let Some(tab) = state.tabs.remove(&(session, kind)) {
            let _ = state.browser.close_target(&tab.target_id);
        }
    }
    state.challenges.retain(|(s, _)| *s != session);
    state.search_locks.remove(&session);
}

pub(crate) fn is_challenged(session: i64, provider: &str) -> bool {
    STATE
        .lock()
        .as_ref()
        .is_some_and(|s| s.challenges.contains(&(session, provider.to_string())))
}

pub(crate) fn mark_challenged(session: i64, provider: &str) {
    let mut guard = STATE.lock();
    if let Some(state) = guard.as_mut() {
        state.challenges.insert((session, provider.to_string()));
    }
}

pub(crate) fn clear_challenged(session: i64, provider: &str) {
    let mut guard = STATE.lock();
    if let Some(state) = guard.as_mut() {
        state.challenges.remove(&(session, provider.to_string()));
    }
}

pub(crate) fn reap() {
    let now = Instant::now();
    let mut guard = STATE.lock();
    let Some(state) = guard.as_mut() else {
        return;
    };
    let expired: Vec<(i64, TabKind)> = state
        .tabs
        .iter()
        .filter(|(_, tab)| {
            tab.active == 0 && !tab.reserved && now.saturating_duration_since(tab.last_used) >= IDLE
        })
        .map(|(key, _)| *key)
        .collect();
    let mut closed_search: HashSet<i64> = HashSet::new();
    for key in expired {
        if let Some(tab) = state.tabs.remove(&key) {
            let _ = state.browser.close_target(&tab.target_id);
            if key.1 == TabKind::Search {
                closed_search.insert(key.0);
            }
        }
    }
    for session in closed_search {
        state.challenges.retain(|(s, _)| *s != session);
    }
}
