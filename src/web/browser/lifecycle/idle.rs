//! The idle clock: when the browser has gone unused long enough, close it.

use parking_lot::Mutex;
use std::time::Instant;

use super::STATE;
use super::tabs;
use crate::web::run;

static LAST: Mutex<Option<Instant>> = Mutex::new(None);

pub(crate) fn touch() {
    *LAST.lock() = Some(Instant::now());
}

pub(crate) fn sweep() {
    if run::active() > 0 {
        return;
    }
    let Some(last) = *LAST.lock() else {
        return;
    };
    let mut guard = STATE.lock();
    let Some(live) = guard.as_ref() else {
        return;
    };
    if live.browser.attached() {
        return;
    }
    if last.elapsed() < live.idle {
        return;
    }
    let dead = guard.take();
    drop(guard);
    drop(dead);
    *LAST.lock() = None;
    tabs::clear();
}
