//! Direct-GET provider — Tier 1 transport: plain ureq GET with a
//! browser-shaped UA, plus the bot-wall sniff that decides when to yield
//! to the browser tier.

use anyhow::{Context as _, anyhow};
use std::time::Duration;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36";
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

pub(crate) fn looks_bot_blocked(status: u16, body: &str) -> bool {
    if status == 403 || status == 503 || status == 429 {
        let lower = body.to_lowercase();
        return lower.contains("cloudflare")
            || lower.contains("captcha")
            || lower.contains("challenge")
            || lower.contains("access denied");
    }
    false
}

pub(crate) fn fetch_direct(url: &str) -> anyhow::Result<(String, u16, usize)> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "text/html,application/xhtml+xml")
        .header("Accept-Language", "en-US,en;q=0.9")
        .call()
        .map_err(|e| anyhow!("request failed: {e}"))?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .context("reading body")?;
    let len = body.len();
    Ok((body, status, len))
}

// --- Tier 2: browser ---------------------------------------------------------
//
// One line over the shared session: navigate, wait for real content, hand
// back the rendered DOM. Readiness = the converted markdown stops looking
