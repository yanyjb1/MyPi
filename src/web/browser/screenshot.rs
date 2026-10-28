use anyhow::{Context as _, Result};
use std::time::{SystemTime, UNIX_EPOCH};

use super::page::Page;
use super::session;
use crate::server::config::BrowserConfig;
use crate::web::WebArgs;

pub(crate) fn run(args: &WebArgs, cfg: &BrowserConfig, session: i64) -> Result<String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("mypi-web-{session}-{millis}.png"));
    session::with_work(session, cfg, args.reserve, |page: &Page| {
        let bytes = page.screenshot(args.full_page)?;
        std::fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
        Ok(format!(
            "screenshot: {} bytes -> {}",
            bytes.len(),
            path.display()
        ))
    })
}
