//! Op dispatch under the process-wide permit: at most two operations run at once.

use anyhow::{Result, anyhow, bail};
use parking_lot::{Condvar, Mutex};

use super::ops;
use super::search;
use crate::web::WebArgs;

static ACTIVE: Mutex<usize> = Mutex::new(0);
static WAKE: Condvar = Condvar::new();

struct Permit;

impl Drop for Permit {
    fn drop(&mut self) {
        *ACTIVE.lock() -= 1;
        WAKE.notify_one();
    }
}

fn permit() -> Permit {
    let mut active = ACTIVE.lock();
    while *active >= 2 {
        WAKE.wait(&mut active);
    }
    *active += 1;
    Permit
}

/// How many operations are in flight; the idle sweep leaves a busy browser alone.
pub(crate) fn active() -> usize {
    *ACTIVE.lock()
}

/// One operation under the permit; the op names which.
pub fn run(args: &WebArgs, session: i64) -> Result<String> {
    let _permit = permit();
    match args.op.as_str() {
        "read" => ops::read::run(args, session),
        "search" => {
            let query = args
                .query
                .as_deref()
                .ok_or_else(|| anyhow!("`search` requires `query`"))?;
            let provider = args
                .provider
                .as_deref()
                .ok_or_else(|| anyhow!("`search` requires `provider`"))?;
            search::run(session, query, provider, args.pages)
        }
        "open" => ops::open::run(args, session),
        "click" | "fill" | "select" | "scroll" | "eval" => ops::drive::run(args, session),
        "screenshot" => ops::screenshot::run(args, session),
        other => bail!(
            "unknown op {other:?}; read|search|open|click|fill|select|scroll|eval|screenshot"
        ),
    }
}
