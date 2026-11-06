//! Tab pool: one target per session and kind, leased for one operation at a time.

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
    reserved: bool,
}

struct Pool {
    tabs: HashMap<(i64, TabKind), Tab>,
    locks: HashMap<i64, Arc<Mutex<()>>>,
}

static POOL: Mutex<Option<Pool>> = Mutex::new(None);

fn ensure(pool: &mut Option<Pool>) -> &mut Pool {
    pool.get_or_insert_with(|| Pool {
        tabs: HashMap::new(),
        locks: HashMap::new(),
    })
}

struct Lease {
    key: (i64, TabKind),
    page: Page,
}

impl Drop for Lease {
    fn drop(&mut self) {
        {
            let mut guard = POOL.lock();
            if let Some(pool) = guard.as_mut()
                && let Some(tab) = pool.tabs.get_mut(&self.key)
            {
                tab.active = tab.active.saturating_sub(1);
                tab.last_used = Instant::now();
                if self.key.1 == TabKind::Work {
                    tab.reserved = false;
                }
            }
        }
        lifecycle::touch();
    }
}

fn with_tab<T>(
    port: u16,
    session: i64,
    kind: TabKind,
    reserve: bool,
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let page = {
        let mut guard = POOL.lock();
        let pool = ensure(&mut guard);
        let client = Client::new(port);
        let live = client.targets()?;
        let key = (session, kind);
        let target = match pool
            .tabs
            .get(&key)
            .and_then(|tab| live.iter().find(|t| t.id == tab.target_id).cloned())
        {
            Some(target) => target,
            None => {
                pool.tabs.remove(&key);
                let target = client.create_target("about:blank")?;
                pool.tabs.insert(
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
        let tab = pool.tabs.get_mut(&key).expect("the tab was inserted above");
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
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let lock = {
        let mut guard = POOL.lock();
        ensure(&mut guard)
            .locks
            .entry(session)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _serial = lock.lock();
    let port = lifecycle::port()?;
    with_tab(port, session, TabKind::Search, false, f)
}

pub(crate) fn with_work<T>(
    session: i64,
    f: impl FnOnce(&Page) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let port = lifecycle::port()?;
    with_tab(port, session, TabKind::Work, true, f)
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
        }
        pool.locks.remove(&session);
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
                tab.active == 0
                    && !tab.reserved
                    && now.saturating_duration_since(tab.last_used) >= IDLE
            })
            .map(|(key, _)| *key)
            .collect();
        keys.into_iter()
            .filter_map(|key| pool.tabs.remove(&key).map(|tab| tab.target_id))
            .collect()
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
    }
}
