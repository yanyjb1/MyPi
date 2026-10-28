use anyhow::{Context as _, anyhow};
use parking_lot::Mutex;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::cdp::{Cdp, Target};
use super::process::Browser;

pub(crate) const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const RENDER_WAIT: Duration = Duration::from_secs(15);
pub(crate) const POLL_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone)]
pub(crate) struct Page {
    port: u16,
    target_id: String,
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

    pub(crate) fn wait(
        &self,
        budget: Duration,
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
                Err(e) if is_reset(&e) => {}
                Err(e) => return Err(e),
            }
            if Instant::now() > deadline {
                return Ok(last);
            }
            std::thread::sleep(Duration::from_millis(400));
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
                Err(e) if is_reset(&e) => {}
                Err(e) => return Err(e),
            }
        }
        let mut guard = self.cdp.lock();
        *guard = self.connect_fresh()?;
        f(&mut guard)
    }

    fn connect_fresh(&self) -> anyhow::Result<Cdp> {
        let browser =
            Browser::attach(Path::new(""), self.port).context("re-attaching to browser")?;
        let target = browser
            .targets()?
            .into_iter()
            .find(|t| t.id == self.target_id)
            .ok_or_else(|| anyhow!("tab {} vanished", self.target_id))?;
        Cdp::connect(&target.ws_url)
    }
}

pub(crate) fn connect(port: u16, target: &Target) -> anyhow::Result<Page> {
    let cdp = Cdp::connect(&target.ws_url)?;
    Ok(Page {
        port,
        target_id: target.id.clone(),
        cdp: Arc::new(Mutex::new(cdp)),
    })
}

fn is_reset(e: &anyhow::Error) -> bool {
    e.to_string().contains("connection reset")
}
