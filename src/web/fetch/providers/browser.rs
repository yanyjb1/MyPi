//! Browser provider — Tier 2 transport: render through the shared
//! session and pull the DOM that survives anti-bot pages.

use super::super::engine::{html_to_markdown, is_js_shell};
use crate::ai::config::BrowserConfig;
use std::time::Duration;

const RENDER_WAIT: Duration = Duration::from_secs(12);

// One line over the shared session: navigate, wait for real content, hand
// back the rendered DOM. Readiness = the converted markdown stops looking
// like a JS shell (or the budget expires and we take what we have).
pub(crate) fn fetch_via_browser(url: &str, cfg: &BrowserConfig) -> anyhow::Result<String> {
    let url_owned = url.to_string();
    // Readiness is conversion semantics, not raw size: a 3.5 KB meta shell
    // (YouTube) passes any byte threshold instantly. The page counts as
    // ready only when its DOM converts to non-shell markdown.
    crate::web::utils::session::with_transient(cfg, url, |p| {
        p.navigate(&url_owned)?;
        p.wait(RENDER_WAIT, |html| {
            match html_to_markdown(html) {
                Ok(md) if is_js_shell(html.len(), &md) => None,
                Ok(_) => Some(Ok(())),
                Err(_) => None, // conversion can't run mid-navigation
            }
        })
    })
}
