// One target's CDP surface: navigate, evaluate, read, wait, capture.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use parking_lot::Mutex;
use serde_json::json;

use super::devtools::{Cdp, Client, Knocker, Target};
use crate::server::platform::stop::{Hook, Stop};
use crate::web::halt;

const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const RENDER_WAIT: Duration = Duration::from_secs(15);
const POLL_TIMEOUT: Duration = Duration::from_secs(8);

// How long one look at the page waits before it looks again.
const LOOK_AGAIN: Duration = Duration::from_millis(400);

pub(crate) struct Page {
    port: u16,
    target_id: String,
    knocker: Arc<Knocker>, // one page, one stop: every connection this page opens answers the same flag
    cdp: Arc<Mutex<Cdp>>,
}

impl Page {
    pub(crate) fn navigate(&self, url: &str) -> anyhow::Result<()> {
        self.resilient(|c| c.navigate(url, NAVIGATE_TIMEOUT).map(|_| ()))
    }

    pub(crate) fn evaluate(&self, expression: &str, timeout: Duration) -> anyhow::Result<String> {
        self.resilient(|c| {
            let v = c.evaluate(expression, timeout)?;
            Ok(match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            })
        })
    }

    pub(crate) fn dom_html(&self) -> anyhow::Result<String> {
        self.resilient(|c| c.dom_html(POLL_TIMEOUT))
    }

    // The document's current URL.
    pub(crate) fn url(&self) -> anyhow::Result<String> {
        self.evaluate("location.href", POLL_TIMEOUT)
    }

    // Raise the tab, and the window that holds it.
    pub(crate) fn bring_to_front(&self) -> anyhow::Result<()> {
        self.resilient(|c| {
            c.call("Page.bringToFront", json!({}), POLL_TIMEOUT)
                .map(|_| ())
        })
    }

    // Hand this page to the stop: a stop ends whatever wait it is holding, wherever that wait sits.
    pub(crate) fn end_on_stop(&self, stop: &Arc<Stop>) -> Hook {
        let knocker = self.knocker.clone();
        stop.on_stop(move || knocker.abort())
    }

    // Read the page until `ready` accepts it. A stop ends the loop with whatever HTML was last in hand.
    pub(crate) fn wait(
        &self,
        budget: Duration,
        stop: &Arc<Stop>,
        ready: impl Fn(&str) -> Option<anyhow::Result<()>>,
    ) -> anyhow::Result<String> {
        let deadline = Instant::now() + budget;
        let mut last = String::new();
        loop {
            match self.dom_html() {
                Ok(html) => match ready(&html) {
                    Some(Ok(())) => return Ok(html),
                    Some(Err(e)) => return Err(e),
                    None => last = html,
                },
                Err(e) if halt::is(&e) => return hand_over(last, e),
                Err(e) if is_reset(&e) => {}
                Err(e) => return Err(e),
            }
            if Instant::now() > deadline {
                return Ok(last);
            }
            if !stop.pause(LOOK_AGAIN) {
                return hand_over(last, halt::Stopped.into());
            }
        }
    }

    pub(crate) fn screenshot(&self, full_page: bool) -> anyhow::Result<Vec<u8>> {
        let mut params = json!({"format": "png"});
        if full_page {
            params["captureBeyondViewport"] = json!(true);
        }
        let data = self.resilient(|c| {
            let r = c.call(
                "Page.captureScreenshot",
                params.clone(),
                Duration::from_secs(15),
            )?;
            r.get("data")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow!("no screenshot data"))
        })?;
        crate::web::url::base64_decode(&data)
    }

    fn resilient<T>(&self, f: impl Fn(&mut Cdp) -> anyhow::Result<T>) -> anyhow::Result<T> {
        {
            let mut guard = self.cdp.lock();
            match f(&mut guard) {
                Ok(v) => return Ok(v),
                // A stopped call is over: reconnecting would only start the wait again.
                Err(e) if halt::is(&e) || !is_reset(&e) => return Err(e),
                Err(_) => {}
            }
        }
        if self.knocker.aborted() {
            return Err(halt::Stopped.into());
        }
        let mut guard = self.cdp.lock();
        *guard = self.connect_fresh()?;
        f(&mut guard)
    }

    fn connect_fresh(&self) -> anyhow::Result<Cdp> {
        let target = Client::new(self.port)
            .targets()?
            .into_iter()
            .find(|t| t.id == self.target_id)
            .ok_or_else(|| anyhow!("tab {} vanished", self.target_id))?;
        Cdp::connect(&target.ws_url, self.knocker.clone())
    }
}

pub(crate) fn connect(port: u16, target: &Target) -> anyhow::Result<Page> {
    let knocker = Knocker::new();
    let cdp = Cdp::connect(&target.ws_url, knocker.clone())?;
    Ok(Page {
        port,
        target_id: target.id.clone(),
        knocker,
        cdp: Arc::new(Mutex::new(cdp)),
    })
}

fn is_reset(e: &anyhow::Error) -> bool {
    e.to_string().contains("connection reset")
}

// A wait cut short answers what it already read; an empty one answers the stop itself.
fn hand_over(last: String, stopped: anyhow::Error) -> anyhow::Result<String> {
    match last.is_empty() {
        true => Err(stopped),
        false => Ok(last),
    }
}
