// Waiting for a human to clear a verification challenge.

use std::sync::Arc;

use anyhow::{Result, bail};

use super::page::Page;
use super::{lifecycle, tabs};
use crate::server::platform::stop::Stop;

// How often the page is read again, and how long the whole wait may last.
const POLL: std::time::Duration = std::time::Duration::from_secs(30);
const WAIT: std::time::Duration = std::time::Duration::from_secs(300);

// Poll the session's search tab until `done` accepts the page, or five minutes pass; a stop ends the wait at once.
pub(crate) fn await_human(
    session: i64,
    stop: &Arc<Stop>,
    done: impl Fn(&str, &str) -> bool,
) -> Result<()> {
    tabs::with_search(session, |page: &Page| {
        if lifecycle::bring_to_front() {
            let _ = page.bring_to_front();
        }
        let deadline = std::time::Instant::now() + WAIT;
        loop {
            let passed = match (page.url(), page.dom_html()) {
                (Ok(url), Ok(html)) => done(&url, &html),
                _ => false,
            };
            if passed {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                bail!(
                    "the human verification was not completed within 5 minutes; the user may be \
                     away or may not have noticed the browser window. Ask the user to finish it, \
                     then retry."
                );
            }
            if !stop.pause(POLL) {
                bail!("the human verification was not completed; the turn was stopped");
            }
        }
    })
}
