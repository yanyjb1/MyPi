//! Waiting for a human to clear a verification challenge.

use anyhow::{Result, bail};
use std::time::{Duration, Instant};

use super::page::Page;
use super::{lifecycle, tabs};

const POLL: Duration = Duration::from_secs(30);
const WAIT: Duration = Duration::from_secs(300);

/// Poll the session's search tab until `done` accepts the page, or five minutes pass.
pub(crate) fn await_human(session: i64, done: impl Fn(&str, &str) -> bool) -> Result<()> {
    tabs::with_search(session, |page: &Page| {
        if lifecycle::bring_to_front() {
            let _ = page.bring_to_front();
        }
        let deadline = Instant::now() + WAIT;
        loop {
            let passed = match (page.url(), page.dom_html()) {
                (Ok(url), Ok(html)) => done(&url, &html),
                _ => false,
            };
            if passed {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "the human verification was not completed within 5 minutes; the user may be \
                     away or may not have noticed the browser window. Ask the user to finish it, \
                     then retry."
                );
            }
            std::thread::sleep(POLL);
        }
    })
}
