//! The DevTools HTTP endpoints: which targets exist, and how to add or drop one.

mod cdp;
mod target;

pub(crate) use cdp::Cdp;
pub(crate) use target::Target;

use anyhow::Context as _;
use std::time::Duration;

pub(crate) struct Client {
    port: u16,
}

impl Client {
    pub(crate) fn new(port: u16) -> Self {
        Self { port }
    }

    /// True when something accepts TCP on 127.0.0.1:port.
    pub(crate) fn port_alive(port: u16) -> bool {
        std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_millis(500),
        )
        .is_ok()
    }

    /// GET /json/list, parsed.
    pub(crate) fn targets(&self) -> anyhow::Result<Vec<Target>> {
        let url = format!("http://127.0.0.1:{}/json/list", self.port);
        let body: Vec<Target> = ureq::get(&url)
            .header("Host", &format!("127.0.0.1:{}", self.port))
            .call()
            .map_err(|e| anyhow::anyhow!("json/list failed: {e}"))?
            .body_mut()
            .read_json()
            .context("parsing /json/list")?;
        Ok(body)
    }

    /// PUT /json/new?url=… — open a fresh tab, return its target.
    /// (Chromium ≥ 111 requires PUT; GET returns 405.)
    pub(crate) fn create_target(&self, url: &str) -> anyhow::Result<Target> {
        let endpoint = format!(
            "http://127.0.0.1:{}/json/new?{}",
            self.port,
            crate::web::url::encode_query(url)
        );
        let mut resp = ureq::put(&endpoint)
            .header("Host", &format!("127.0.0.1:{}", self.port))
            .send_empty()
            .map_err(|e| anyhow::anyhow!("json/new failed: {e}"))?;
        let body: Target = resp
            .body_mut()
            .read_json()
            .context("parsing /json/new response")?;
        Ok(body)
    }

    /// GET /json/close/<id> — close a tab by target id.
    pub(crate) fn close_target(&self, id: &str) -> anyhow::Result<()> {
        let endpoint = format!("http://127.0.0.1:{}/json/close/{}", self.port, id);
        ureq::get(&endpoint)
            .header("Host", &format!("127.0.0.1:{}", self.port))
            .call()
            .map_err(|e| anyhow::anyhow!("json/close failed: {e}"))?;
        Ok(())
    }
}
