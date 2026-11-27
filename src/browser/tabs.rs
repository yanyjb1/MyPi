//! Tab pool: one target per session and kind, one call at a time.
//!
//! A session owns at most two tabs — search and work. The slot lock is held for the whole call,
//! so two calls in one session queue on the tab instead of interleaving on it, and the global
//! pool lock only ever guards the maps: `/json/list`, `/json/new` and the CDP handshake happen
//! outside it.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::devtools::Client;
use super::lifecycle;
use super::page::{self, Page};

/// A tab with no active call closes after this long.
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
}

struct Pool {
    tabs: HashMap<(i64, TabKind), Tab>,
    slots: HashMap<(i64, TabKind), Arc<Mutex<()>>>,
    /// The `about:blank` the launch argv opens has been reclaimed.
    launch_tab: bool,
}

static POOL: Mutex<Option<Pool>> = Mutex::new(None);

fn ensure(pool: &mut Option<Pool>) -> &mut Pool {
    pool.get_or_insert_with(|| Pool {
        tabs: HashMap::new(),
        slots: HashMap::new(),
        launch_tab: false,
    })
}

impl Pool {
    fn slot(&mut self, key: (i64, TabKind)) -> Arc<Mutex<()>> {
        self.slots
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    fn mark(&mut self, key: (i64, TabKind), target_id: &str) {
        let tab = self.tabs.entry(key).or_insert_with(|| Tab {
            target_id: target_id.to_string(),
            last_used: Instant::now(),
            active: 0,
        });
        if tab.target_id != target_id {
            tab.target_id = target_id.to_string();
            tab.active = 0;
        }
        tab.last_used = Instant::now();
    }

    fn live_id(&self, key: (i64, TabKind)) -> Option<&str> {
        self.tabs.get(&key).map(|tab| tab.target_id.as_str())
    }

    fn enter(&mut self, key: (i64, TabKind)) {
        if let Some(tab) = self.tabs.get_mut(&key) {
            tab.active += 1;
            tab.last_used = Instant::now();
        }
    }
}

/// One call's claim on a tab. Taken before the CDP handshake, so a tab being connected to is
/// never idle-reaped underneath the call.
struct Active {
    key: (i64, TabKind),
}

impl Drop for Active {
    fn drop(&mut self) {
        {
            let mut guard = POOL.lock();
            if let Some(pool) = guard.as_mut()
                && let Some(tab) = pool.tabs.get_mut(&self.key)
            {
                tab.active = tab.active.saturating_sub(1);
                tab.last_used = Instant::now();
            }
        }
        lifecycle::touch();
    }
}

struct Lease {
    _active: Active,
    page: Page,
}

/// Close the `about:blank` the launch argv opened, once, so a fresh browser shows the session's
/// own tabs and nothing else. Never touches an attached browser: its tabs are the user's.
fn reclaim_launch_tab(client: &Client) {
    let first = {
        let mut guard = POOL.lock();
        let pool = ensure(&mut guard);
        if pool.launch_tab {
            false
        } else {
            pool.launch_tab = true;
            true
        }
    };
    if !first || !lifecycle::owned() {
        return;
    }
    let Ok(live) = client.targets() else {
        return;
    };
    let known: Vec<String> = {
        let guard = POOL.lock();
        guard
            .as_ref()
            .map(|pool| {
                pool.tabs
                    .values()
                    .map(|tab| tab.target_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    };
    for target in live {
        if target.url == "about:blank" && !known.contains(&target.id) {
            let _ = client.close_target(&target.id);
        }
    }
}

fn with_tab<T>(
    session: i64,
    kind: TabKind,
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let key = (session, kind);
    let slot = {
        let mut guard = POOL.lock();
        ensure(&mut guard).slot(key)
    };
    let _serial = slot.lock();

    let port = lifecycle::port()?;
    let client = Client::new(port);
    let live = client.targets()?;

    let known = {
        let guard = POOL.lock();
        let wanted = guard
            .as_ref()
            .and_then(|pool| pool.live_id(key))
            .map(str::to_string);
        wanted.and_then(|id| live.iter().find(|t| t.id == id).cloned())
    };
    let target = match known {
        Some(target) => target,
        None => {
            // A vanished tab is replaced, never reused: the id in the pool is the only tab this
            // session may drive.
            let created = client.create_target("about:blank")?;
            {
                let mut guard = POOL.lock();
                if let Some(pool) = guard.as_mut() {
                    pool.mark(key, &created.id);
                }
            }
            reclaim_launch_tab(&client);
            created
        }
    };

    if let Some(pool) = POOL.lock().as_mut() {
        pool.enter(key);
    }
    let active = Active { key };
    let page = page::connect(port, &target)?;
    let lease = Lease {
        _active: active,
        page,
    };
    f(&lease.page)
}

pub(crate) fn with_search<T>(
    session: i64,
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    with_tab(session, TabKind::Search, f)
}

pub(crate) fn with_work<T>(
    session: i64,
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    with_tab(session, TabKind::Work, f)
}

pub fn close_session(session: i64) {
    let closed: Vec<String> = {
        let mut guard = POOL.lock();
        let Some(pool) = guard.as_mut() else {
            return;
        };
        let mut closed = Vec::new();
        for kind in [TabKind::Search, TabKind::Work] {
            if let Some(tab) = pool.tabs.remove(&(session, kind)) {
                closed.push(tab.target_id);
            }
            pool.slots.remove(&(session, kind));
        }
        closed
    };
    if let Some(port) = lifecycle::existing_port() {
        let client = Client::new(port);
        for id in closed {
            let _ = client.close_target(&id);
        }
    }
}

/// Close tabs that no call has touched for `IDLE`.
pub(crate) fn reap() {
    let now = Instant::now();
    let expired: Vec<String> = {
        let mut guard = POOL.lock();
        let Some(pool) = guard.as_mut() else {
            return;
        };
        let keys: Vec<(i64, TabKind)> = pool
            .tabs
            .iter()
            .filter(|(_, tab)| {
                tab.active == 0 && now.saturating_duration_since(tab.last_used) >= IDLE
            })
            .map(|(key, _)| *key)
            .collect();
        let mut gone = Vec::new();
        for key in keys {
            if let Some(tab) = pool.tabs.remove(&key) {
                gone.push(tab.target_id);
            }
            pool.slots.remove(&key);
        }
        gone
    };
    let Some(port) = lifecycle::existing_port() else {
        clear();
        return;
    };
    let client = Client::new(port);
    for id in expired {
        let _ = client.close_target(&id);
    }
}

/// Forget every tab: the browser they lived in is gone.
pub(crate) fn clear() {
    if let Some(pool) = POOL.lock().as_mut() {
        pool.tabs.clear();
        pool.launch_tab = false;
    }
}
