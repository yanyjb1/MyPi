//! One provider per call: pages 1..=N through the search tab, until a page comes back empty.

use anyhow::{Result, anyhow, bail};
use std::collections::HashSet;

use self::hit::{Hit, dedup_key};
use self::provider::SearchProvider;
use super::browser::page::{Page, RENDER_WAIT};
use super::browser::{tabs, verify};

mod hit;
mod provider;
mod providers;

pub(crate) fn run(session: i64, query: &str, provider_name: &str, page: u32) -> Result<String> {
    if page == 0 {
        bail!("page must be >= 1");
    }
    let provider = provider::get(provider_name).ok_or_else(|| {
        anyhow!(
            "unknown provider {provider_name:?}; google|bing|duckduckgo|ecosia|startpage|mojeek"
        )
    })?;
    let name = provider.name();

    let mut hits: Vec<Hit> = Vec::new();
    let mut seen = HashSet::new();
    let mut current = 1u32;
    while current <= page {
        let url = provider.page_url(query, current);
        let mut html = fetch(session, &url, provider)?;

        if provider.challenge(&url, &html).is_some() {
            verify::await_human(session, |challenge_url, challenge_html| {
                provider.challenge(challenge_url, challenge_html).is_none()
            })
            .map_err(|e| anyhow!("{name}: {e}"))?;
            html = fetch(session, &url, provider)?;
            if provider.challenge(&url, &html).is_some() {
                bail!(
                    "{name}: the verification page is still there. Ask the user to finish it, \
                     then retry."
                );
            }
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

fn fetch(session: i64, url: &str, provider: &'static dyn SearchProvider) -> Result<String> {
    tabs::with_search(session, |page: &Page| {
        page.navigate(url)?;
        page.wait(RENDER_WAIT, |html| {
            if provider.challenge(url, html).is_some() {
                return Some(Ok(()));
            }
            if !provider.parse(html).is_empty() {
                return Some(Ok(()));
            }
            None
        })
    })
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
