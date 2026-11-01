//! Op dispatch under the process-wide permit: at most two operations run at once.

use anyhow::{Result, bail};
use std::sync::{Condvar, Mutex};

use super::WebArgs;
use super::{browser, search};

static ACTIVE: Mutex<usize> = Mutex::new(0);
static WAKE: Condvar = Condvar::new();

struct Permit;

impl Drop for Permit {
    fn drop(&mut self) {
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        *active = (*active).saturating_sub(1);
        WAKE.notify_one();
    }
}

fn permit() -> Permit {
    let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    while *active >= 2 {
        active = WAKE.wait(active).unwrap_or_else(|e| e.into_inner());
    }
    *active += 1;
    Permit
}

/// One operation under the permit; the op names which.
pub fn run(args: &WebArgs, session: i64) -> Result<String> {
    let _permit = permit();
    match args.op.as_str() {
        "read" => browser::read(args, session),
        "search" => {
            let query = args
                .query
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("`search` requires `query`"))?;
            let provider = args
                .provider
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("`search` requires `provider`"))?;
            search::run(session, query, provider, args.page)
        }
        "open" => browser::open(args, session),
        "click" | "fill" | "press" | "select" | "scroll" | "eval" => {
            browser::actions(args, session)
        }
        "net" => browser::net(args, session),
        "screenshot" => browser::screenshot(args, session),
        other => bail!(
            "unknown op {other:?}; read|search|open|click|fill|press|select|scroll|eval|net|screenshot"
        ),
    }
}
