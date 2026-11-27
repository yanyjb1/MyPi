// Op dispatch under the process-wide permit: at most two operations run at once.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use parking_lot::{Condvar, Mutex};

use super::ops;
use super::search;
use crate::browser::WebArgs;
use crate::platform::stop::Stop;
use crate::server::config::rules::Rules;

static ACTIVE: Mutex<usize> = Mutex::new(0);
static WAKE: Condvar = Condvar::new();

struct Permit;

impl Drop for Permit {
    fn drop(&mut self) {
        *ACTIVE.lock() -= 1;
        WAKE.notify_one();
    }
}

// One operation's place in the queue; a stop wakes the queue, so a stopped call leaves without waiting its turn.
fn permit(stop: &Arc<Stop>) -> Option<Permit> {
    // Notifying while holding the queue is what makes it race-free: a stop that registers a waiter still reaches it.
    let _hook = stop.on_stop(|| {
        let _queue = ACTIVE.lock();
        WAKE.notify_all();
    });
    let mut active = ACTIVE.lock();
    while *active >= 2 {
        if stop.is_stopped() {
            return None;
        }
        WAKE.wait(&mut active);
    }
    if stop.is_stopped() {
        return None;
    }
    *active += 1;
    Some(Permit)
}

// How many operations are in flight; the idle sweep leaves a busy browser alone.
pub(crate) fn active() -> usize {
    *ACTIVE.lock()
}

// One operation under the permit; the op names which.
pub fn run(args: &WebArgs, session: i64, rules: &Rules, stop: &Arc<Stop>) -> Result<String> {
    let Some(_permit) = permit(stop) else {
        bail!("the call was stopped before it started");
    };
    match args.op.as_str() {
        "read" => ops::read::run(args, session, rules, stop),
        "search" => {
            let query = args
                .query
                .as_deref()
                .ok_or_else(|| anyhow!("`search` requires `query`"))?;
            let provider = args
                .provider
                .as_deref()
                .ok_or_else(|| anyhow!("`search` requires `provider`"))?;
            search::run(session, query, provider, args.pages, stop)
        }
        "open" => ops::open::run(args, session, stop),
        "click" | "fill" | "select" | "scroll" | "eval" => ops::drive::run(args, session, stop),
        "screenshot" => ops::screenshot::run(args, session, stop),
        other => {
            bail!("unknown op {other:?}; read|search|open|click|fill|select|scroll|eval|screenshot")
        }
    }
}
