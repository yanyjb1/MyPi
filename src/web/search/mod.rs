use anyhow::{Result, anyhow, bail};
use std::collections::HashSet;
use std::time::{Duration, Instant};

use self::hit::{Hit, dedup_key};
use self::provider::SearchProvider;
use super::browser;
use crate::server::config::BrowserConfig;

mod hit;
mod provider;
mod providers;

const VERIFY_INTERVAL: Duration = Duration::from_secs(60);
const VERIFY_TIMEOUT: Duration = Duration::from_secs(300);

pub(crate) fn run(
    session: i64,
    cfg: &BrowserConfig,
    query: &str,
    provider_name: &str,
    page: u32,
) -> Result<String> {
    if page == 0 {
        bail!("page must be >= 1");
    }
    let provider = provider::get(provider_name).ok_or_else(|| {
        anyhow!(
            "unknown provider {provider_name:?}; google|bing|duckduckgo|ecosia|startpage|mojeek"
        )
    })?;
    let name = provider.name();
    let mut verified = false;
    if browser::is_challenged(session, name) {
        browser::ensure_headed(cfg).map_err(|e| anyhow!("{name}: {e}"))?;
        verify(session, cfg, provider, query)?;
        browser::clear_challenged(session, name);
        verified = true;
    }

    let mut hits: Vec<Hit> = Vec::new();
    let mut seen = HashSet::new();
    let mut current = 1u32;
    while current <= page {
        let url = provider.page_url(query, current);
        let html = browser::with_search(session, cfg, |page| {
            page.navigate(&url)?;
            page.wait(browser::RENDER_WAIT, |html| {
                if provider.challenge(&url, html).is_some() {
                    return Some(Ok(()));
                }
                if !provider.parse(html).is_empty() {
                    return Some(Ok(()));
                }
                None
            })
        })?;

        if provider.challenge(&url, &html).is_some() {
            if !browser::is_challenged(session, name) {
                browser::mark_challenged(session, name);
                if !browser::has_display() {
                    bail!(
                        "{name}: human verification required at {url}; no display is available for a headed browser."
                    );
                }
                bail!(
                    "{name}: human verification required. Retry the same provider to open a headed browser, or switch provider."
                );
            }
            if verified {
                bail!("{name}: verification did not clear; retry or switch provider.");
            }
            browser::ensure_headed(cfg).map_err(|e| anyhow!("{name}: {e}"))?;
            verify(session, cfg, provider, query)?;
            browser::clear_challenged(session, name);
            verified = true;
            hits.clear();
            seen.clear();
            current = 1;
            continue;
        }

        let parsed = provider.parse(&html);
        if parsed.is_empty() {
            break;
        }
        for hit in parsed {
            if seen.insert(dedup_key(&hit.url)) {
                hits.push(hit);
            }
        }
        current += 1;
    }

    Ok(render(name, page, &hits))
}

fn verify(
    session: i64,
    cfg: &BrowserConfig,
    provider: &'static dyn SearchProvider,
    query: &str,
) -> Result<()> {
    let url = provider.page_url(query, 1);
    browser::with_search(session, cfg, |page| page.navigate(&url))?;
    let deadline = Instant::now() + VERIFY_TIMEOUT;
    loop {
        let html = browser::with_search(session, cfg, |page| page.dom_html())?;
        if provider.challenge(&url, &html).is_none() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "{}: verification not completed within 5 minutes. Browser is headed; retry or switch provider.",
                provider.name()
            );
        }
        std::thread::sleep(VERIFY_INTERVAL);
    }
}

fn render(name: &str, page: u32, hits: &[Hit]) -> String {
    let mut out = format!("{name}: pages 1-{page}, {} results\n", hits.len());
    for (i, hit) in hits.iter().enumerate() {
        out.push_str(&format!("\n{}. {}\n   {}\n", i + 1, hit.title, hit.url));
        if !hit.snippet.is_empty() {
            out.push_str(&format!("   {}\n", hit.snippet));
        }
    }
    out
}
